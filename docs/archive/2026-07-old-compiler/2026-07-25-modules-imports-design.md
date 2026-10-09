# Modules & imports: design

## Context

Kestrel is single-file today: `kestrelc` takes exactly one `.kes` path,
parses it into one `Program`, and runs the whole pipeline
(resolve → typecheck → purity → codegen) over it. This blocks anything
that needs to span more than one file. This design adds a minimal
`use` system: one module per file, resolved by bare name in the same
directory as the importing file, merged into a single `Program` before
compilation — no new linking model, no changes to codegen's
single-whole-program assumptions (memoization, JIT watch, profile-guided
inlining all keep working exactly as they do today, because there's
still only ever one `Program` by the time they run).

## Syntax

```
use math_utils;
use sqrt, abs from geometry;
use {
    math_utils;
    sqrt, abs from geometry;
}
```

- Bare (`use math_utils;`) → qualified access only: `math_utils.sqrt(x)`.
- `from` (`use sqrt, abs from geometry;`) → those names become callable
  unqualified: `sqrt(x)`.
- `use { ... }` is pure sugar for several `use` lines grouped together —
  identical semantics either way.
- `use` is a new top-level `Program` item (not a statement inside a
  function body) and **must appear before any `fn`/`struct` declaration**
  in the file — a `use` after any other top-level item is a parse error.

## Module resolution

A module is exactly one file; its name is its filename stem
(`math_utils.kes` → module `math_utils`). `use math_utils;` resolves by
looking for `math_utils.kes` in **the same directory as the file
containing the `use` statement** — not the entry file's directory. This
applies transitively: if `geometry.kes` itself has `use vectors;`,
`vectors.kes` is looked up next to `geometry.kes`, wherever that is.

**Known v1 limitation, stated explicitly:** there is no path syntax in
`use` — a module can only ever be found in the same directory as
whatever imports it. A multi-file project must keep all
interdependent files flat in one folder for now. Reaching into a
subfolder is a real, separate future feature (e.g. `use "sub/mod";`),
not part of this design.

A filename that isn't a valid Kestrel identifier (e.g. `math-utils.kes`,
hyphenated) can never be validly named in a `use` statement — this
surfaces naturally as "file not found" (there is no valid `use` spelling
that would look for a hyphenated name) rather than needing a special
validation rule.

Importing the same module more than once (directly and/or transitively)
is **idempotent, not an error** — it's only ever a name collision (see
below) that's rejected, never a redundant `use` on its own.

## Discovery & merge pipeline

Replaces `kestrelc`'s current "parse the one entry file" step:

1. Parse the entry file into a `Program` (as today), plus its `use`
   list (new).
2. For each module referenced (directly or transitively), resolve its
   path (same-directory-as-importer rule above), parse it into its own
   `Program` — recursively collecting *its* `use` list too. Track
   already-parsed modules by resolved absolute path so a module is
   never parsed twice and so an import cycle (A uses B uses A) is
   detected and rejected as a compile error rather than infinite-looping.
3. Merge every module's (and the entry file's own) `fns`/`structs` into
   one combined `Program`, with every function/struct's `Symbol`
   rewritten to a module-qualified internal name (see below). This
   combined `Program` is what resolve/typecheck/purity/codegen actually
   see — identical to today's single-file pipeline from that point on.

## Name resolution & qualification

This is where the real complexity of this feature lives — not parsing.

Every function and struct, from every module including the entry file
itself, is stored internally under a **qualified symbol**:
`{module_name}${original_name}` (`$` chosen because it can't appear in a
source-level identifier, so a qualified symbol can never collide with a
user-typed one). This qualified symbol is what resolve.rs's fn/struct
tables are keyed by, and — critically — **it's also the actual
object-file/linker symbol name codegen exports**, not just an internal
resolver detail. Without this, two unrelated modules each defining a
same-named function (e.g. both have a `helper`) would make Cranelift
try to declare two functions under the identical native symbol, which
is a real crash, not just a Kestrel-level ambiguity.

Within a given file, an unqualified call `foo()` resolves in this
order: (1) a `from`-imported name in scope for this file → that
module's qualified symbol; (2) a function declared in this same file →
this file's own qualified symbol; (3) otherwise, unknown-identifier
error (unchanged from today). A qualified call `module.foo(...)`
(parsed the same postfix shape as the existing `.map(f)` sugar, but
resolved generically: if the target identifier is a name introduced by
a bare `use module;` in this file, `.name(args)` resolves to that
module's qualified symbol instead of being treated as a struct
field/method access) resolves directly to `{module}${foo}`.

**`main` exclusivity:** `well_known::main()` is special-cased throughout
codegen (JIT entry point, exempt from memoization). Only the **entry
file's own** `main` is ever treated as the program's real entry point.
If some other (non-entry) module happens to also declare a function
named `main`, it's an ordinary qualified function like any other —
reachable only via `othermodule.main()` or a `from` import, never
treated as *the* process entry point.

**Collisions are always a compile error**, matching this codebase's
existing "never guess" posture (the type checker, the bounds proofs):
two `from` imports bringing in the same unqualified name, or a `from`
import colliding with a name already declared locally in the importing
file, both reject at compile time with a clear message naming both
sources of the name. Never silent shadowing.

## Explicitly out of scope for this design (stated, not silently dropped)

- **Compile cache correctness.** `kestrelc`'s persistent cache
  (`kestrelc/src/cache.rs`) keys off the entry file's source text only.
  With modules, editing an imported file without touching the entry
  file would produce a stale cache hit — silently running old code.
  Fixing this (hashing every transitively-used file into the cache key)
  is real, necessary follow-up work, not covered by this spec.
- **`kestrelc watch` mode.** Currently only watches the one entry file.
  Editing an imported module during a `watch` session won't trigger a
  recompile until this is addressed separately.
- Both of the above mean: **importing a module and editing it is safe
  in a plain one-shot `kestrelc file.kes` build (no caching involved),
  but is not yet safe under `--cache`/`watch`.** This should be called
  out in user-facing docs once implemented, not just buried here.
- Visibility (`public`/`private`) — every module-level function/struct
  is importable today; there's no way to keep something module-private.
  Separate future design, noted in the earlier session discussion.

## Testing

- Positive: two files in one directory, one `use`-ing the other both
  ways (bare qualified call, and a `from`-imported unqualified call),
  correct output.
- Transitive: A uses B uses C, A calls a `from`-imported name that
  originates in C via B — correct output.
- Cycle: A uses B, B uses A — compile error, not a hang/stack overflow.
- Collision: two `from` imports bringing in the same name — compile
  error naming both modules. A `from` import colliding with a local
  function of the same name — compile error.
- Same-named functions in two unrelated, non-colliding modules (never
  imported together, or only ever accessed qualified) — compiles and
  links cleanly, proving the qualified-symbol export actually prevents
  the object-file-level collision described above.
- `main` in a non-entry module — does not become the entry point;
  calling it via qualified/`from` access still works as an ordinary
  function.
- Missing module file — clear "module not found" compile error naming
  the expected path.
