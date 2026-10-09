# Visibility (pub/private): design

## Context

Modules/imports (`docs/superpowers/specs/2026-07-25-modules-imports-design.md`)
shipped this session with no visibility concept at all: every
module-level `fn`/`struct` is importable from every other module,
whether via `use a, b from module;` or a bare `use module;` +
`module.fn(...)` qualified call. There's no way for a module to keep
an internal helper truly internal. This design adds `pub`.

## Default & syntax

**Private by default.** A bare `fn`/`struct` declaration is only
callable/constructible from within its own module. `pub` makes it
importable from elsewhere:

```
fn internal_helper() { ... }      // private -- only this module can call it
pub fn square(x: i64) -> i64 { ... }
pub struct Point { x: i64, y: i64 }
pub pure fn cube(x: i64) -> i64 { ... }
```

`pub` is a plain prefix keyword in the same modifier slot as `pure` —
no new syntax category (an `@public`-style annotation was considered
and rejected: visibility is a core structural property like `pure`,
not optional compiler guidance like a future `@assume`/`@prove` might
be, so it should read the same way `pure` does, not differently).

This is a behavior change for every module-using example already
written this session (`geometry.kes`'s `square`, etc. would need
`pub` added) — acceptable since modules only shipped this session and
nothing beyond this repo's own examples/tests depends on the old
implicitly-public behavior.

## What's restricted

`pub` gates **cross-module access only** — both access paths
uniformly:
- `use name from module;` (unqualified from-import)
- bare `use module;` + `module.name(...)` (qualified call)

Same-module access (an ordinary unqualified call within the file that
declares the function) is **completely untouched** — it never goes
through any cross-module rename/visibility check at all today, and
this design doesn't add one. Privacy only ever gates the boundary
between modules, never anything inside one.

Scope is declaration-level only, not per-field: a `struct` is visible
as a whole or not; no field-level privacy. Matches YAGNI given structs
are already scope-limited (no arrays-in-structs, no struct returns as
of this session).

## Enforcement

Lives entirely in `modules::merge_modules`, at the exact points that
already validate `from`-imports and populate bare-`use` qualified-call
rename entries (both already look up "does this name exist in the
source module's declared fns/structs" — see the design doc's own
`declared: HashMap<String, (Vec<String>, Vec<String>)>` table). Each
entry there needs to also carry each name's `pub`-ness (e.g.
`(Vec<(String, bool)>, Vec<(String, bool)>)` — name paired with
whether it's public — rather than a bare name list), so both checks
(from-import existence check, bare-use rename-table population) can
also assert the target is public, not just that it exists.

Referencing a private item from another module is a **distinct**
compile error ("'foo' is private to module 'geometry'"), not the
existing "not found in module" message — reusing "not found" would
actively mislead, since the item does exist, it's just inaccessible
from here.

## AST/parser changes

- `Fn` and `StructDecl` (`ast.rs`) each gain a `pub_: bool` field
  (trailing underscore since `pub` is a reserved word in Rust itself).
- Lexer: new `Tok::Pub` for the `"pub"` keyword.
- Parser: `parse_fn_decl`/`parse_struct_decl` each check for a leading
  `Tok::Pub` (before `Tok::Pure`/`Tok::Fn`/`Tok::Struct`, so `pub pure
  fn` and `pure pub fn` — wait, only one order should be accepted, not
  both, to keep the grammar simple and avoid two equally-valid spellings
  for the same thing. **Decision:** `pub` always comes first when both
  are present (`pub pure fn`, not `pure pub fn`) — matches this design's
  own examples above and keeps exactly one canonical spelling.

## Testing

- A private fn's own module can still call it (regression: same-module
  calls completely unaffected).
- A `from`-import of a private fn is a compile error naming it private,
  distinct from "not found".
- A bare-`use` qualified call to a private fn is the same distinct
  error.
- A `pub` fn/struct is importable both ways, unaffected by this
  feature (existing modules integration tests, updated to add `pub`
  where they currently rely on cross-module access, must still pass).
- A private struct is still fully usable by other functions in its own
  module (constructed, field-accessed) even when a `pub` function in
  that same module also exists — visibility never applies within the
  declaring module, only across the module boundary.
- `pub pure fn` and plain `pub fn` both parse; `pure pub fn` (wrong
  keyword order) is a parse error, not silently accepted.
