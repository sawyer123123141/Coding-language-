# Real type checking (call sites, struct fields, return types)

## Status

Implemented directly, not through the full interactive brainstorm —
the user picked a direction ("catch more bugs at compile time," not
"add new types" or "types for codegen speed") then had to step away
mid-session with explicit instruction to keep working. This doc
retroactively records the design for review, following this project's
usual spec convention even though the normal question-by-question
brainstorm didn't run to completion.

## Problem

`typecheck.rs`'s own module doc comment used to say outright: "Deliberately
does NOT check a call site's argument kinds against the callee's declared
parameter type names." A parameter's declared type carried zero
information inside its own function body (`Kind::Unknown`, always).
Struct literal and field-assignment values were never checked against
declared field types. `return` values were never checked against a
declared return type. All four are real, easy-to-hit bug classes a type
checker exists specifically to catch, and none of them were.

## Decision

Add a `type_to_kind(ty: &Type, structs: &HashMap<Symbol, &StructDecl>) -> Kind`
mapping: every integer type name (`i64`, `i32`, `usize`, ...) collapses to
the existing `Kind::Int` (there's still only one runtime integer
representation — this doesn't invent a new distinction, it lets the
checker recognize the one that already exists), `bool`/`str`/`string`
map to `Kind::Bool`/`Kind::Str`, a known struct name maps to
`Kind::Struct(name)`, an array type maps to `Kind::Array`.

Four new checks, all gated on "only fires when both sides are known,
never guesses" — the same posture the original narrower checker
already had:

1. **Call-site argument kinds** vs. the callee's declared param types.
2. **Struct literal field values** vs. the struct's declared field types.
3. **Field-assignment values** (`p.x = value;`) vs. the same declared
   field type.
4. **`return` values** vs. the function's declared return type (bare
   `return;` with no value is deliberately left unchecked — see the
   code comment on why that specific case doesn't generalize safely
   yet).

Two related, cheap wins bundled in since they follow directly from the
same `type_to_kind` machinery:

- A parameter's declared type now seeds its `Kind` inside its own
  function body (previously always `Unknown`) — `fn f(x: i64) { if (x)
  {...} }` now correctly trips the if-condition-must-be-bool check.
- A call expression's own inferred `Kind` now reflects the callee's
  declared return type (previously always `Unknown`) — `let x = f();`
  where `f` returns `bool` now makes `x` a known `Bool` for every
  downstream check, not just a directly-typed literal.

## Explicitly out of scope

No new runtime types (float, a real `i32` vs `i64` distinction) — that
was the "more types to use" direction the user did not pick this round.
No struct-field-type checking beyond scalars (matches structs' existing
scope: scalar fields only, no arrays/nested structs). No JIT changes —
JIT already rejects any program using structs at all, unaffected either
way.

## Files touched

`kestrelc/src/typecheck.rs` (all four checks + `type_to_kind` +
parameter/return-type seeding), `kestrelc/src/main.rs`,
`kestrelc/src/watch.rs`, `kestrelc-devtool/src/runner.rs` (all three
existing `check_types` call sites updated to pass `structs`, already
available at each call site from `resolve::build_struct_table`).

## Testing

8 new unit tests in `typecheck.rs` covering: a rejected mismatched
call-site argument, an accepted matching one, a parameter's type being
visible inside its own body, a rejected struct-literal field value, a
rejected field-assignment value, a rejected return value, and a call's
inferred kind correctly reflecting the callee's declared return type.
Full suite: 158/158 real passes (one known pre-existing flaky timing
test aside), confirmed via a direct rebuild and run, plus two manual
smoke tests (a struct-param function computing correctly, and a
real `f(true)` vs. `f(x: i64)` mismatch producing a clean
`file:line:col:` + caret error).
