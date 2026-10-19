# Interfaces Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add `interface` declarations, `struct X: I` / `enum X: I` conformance, interface types with dynamic dispatch, borrowed and boxed interface values.

**Architecture:** The checker gets `Ty::Iface(IfaceId)` with abstract method `FnInfo`s and a coercion from implementing types. KIR gets `KTy::Iface` = `{data, table}`, `MakeIface { Borrowed | Boxed }` (mirroring `MakeFn`'s stack and heap environments and their invariant) and `CallVirtual`. Codegen emits one method table per (type, interface) as static data holding function addresses. Templates: closures' `MakeFn`/`EnvKind`/`check_stack_envs`/`owned()` conversion (`kir/src/lower/closure.rs`, `env.rs`, `stack_envs.rs`, `collections.rs`), `CallIndirect` (`lower/fnvalue.rs`, `codegen/src/func/fnvalue.rs`), file-name data records (`codegen/src/globals.rs`).

**Tech Stack:** Rust, Cranelift 0.116.1. Tests are Rust unit tests plus `.kes` program and error tests.

**Spec:** `docs/superpowers/specs/2026-10-19-interfaces-design.md`. Every message and behaviour comes from it; the spec wins any conflict.

## Global Constraints

- **Messages:** plain sentences that say what to do, worded exactly as in the spec.
- **File sizes:** under about 400 lines. New code goes in `syntax/src/parser/iface.rs`, `check/src/iface.rs` (declarations, conformance), `check/src/body/iface.rs` (coercion, calls), `kir/src/lower/iface.rs`, `kir/src/lower/tables.rs` (method tables), `codegen/src/func/iface.rs`, `codegen/src/tables.rs`. Split `check/src/collect.rs`, `check/src/tables.rs`, `check/src/body/regions.rs` or `kir/src/lower/helpers.rs` before adding to them if they would pass 400.
- **Every new KIR node** is handled in `visit.rs`, `visit_mut.rs`, `rename.rs`, `inline/descend.rs`, `scalarize.rs`, `promote.rs`, `lower/lines.rs`, `lower/sort.rs` (`builds_memory`, now exhaustive), and codegen `expr.rs`/`stmt.rs`/`aggregate.rs`. `KTy::Iface` gets an arm in every exhaustive `KTy` match (ir.rs, layout.rs, lower/layout.rs, lower/heap.rs, lower/mod.rs `kty`, codegen func/mod.rs, arith.rs, print.rs, export.rs `Other("an interface")`, lower/helpers.rs `fixup`).
- **Runtime functions** need an `RtFuncs` field and `decl` (`codegen/src/rt.rs`), a `jit_symbols()` entry and `#[no_mangle] pub extern "C"`; a runtime callback returns `i8`, never `bool`; a failing one fails through `panic::fail` with `(line, file)`.
- **Test facts:** program tests `kestrel/tests/programs/*.kes` (`// expect:`, `// expect-stderr:`, `// expect-exit:`), run by `cargo test -p kestrel-cli --test programs programs_` (JIT and exe, poison on); error tests `kestrel/tests/errors/*.kes` (`// error <line>: <message>`); multi-file tests in `cli/tests/modules.rs`; memory tests in the style of `cli/tests/text_memory.rs` (`KESTREL_ARENA_REPORT=1` prints `kestrel: program arena N bytes`). No string literals inside `{...}`. Functions other than `main` can't store caller-arena memory into a parameter (use `in x.mem { }`). An `Arena` place can't be assigned a new `Arena`.
- **Speed:** while iterating, run focused targets (`kestrel-check`, `kestrel-kir`, `--test programs`, `--test errors`); run `cargo test --workspace --no-fail-fast` and `cargo clippy --workspace --all-targets -- -D warnings` once before committing. If the user says they are gaming: `CARGO_BUILD_JOBS=2`, `--test-threads=2`, no window/GPU/demo tests, and the full suite is owed.
- **Off-limits:** `kestrel/test.kes`, `voxels.exe`, the line-ending-only change in `kestrel/check/src/body/enums.rs`. Never `git add -A`, `git stash`, `git clean`.
- **Commits:** plain-sentence messages ending with `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`. Work on `main`; the controller pushes.

## Review Focus

1. **A borrowed interface value outliving its call** (stored, returned, copied, spilled or inlined into a `Let` without boxing): the same invariant as stack environments; Task 4 extends the debug check over every program test.
2. **Whole assignment through a borrowed `var` parameter** (`d = Slime()` writing into a caller's `Goblin`): refused in the checker (Task 2), and no other route (passing `var d` on to a concrete `var Goblin` parameter, a `remove`, a `pop`) writes a different type into the caller's place.
3. **Copies sharing memory:** an interface value copied, a list of them filtered, a struct holding one copied; each changed independently under poison (Tasks 3, 4).
4. **Escape and regions with the coerced type:** a plain-numbers struct coerced to an interface and stored out of `arena { }` is refused (Task 2).
5. **Owning implementors:** borrowed is fine, stored is refused at compile time, and the copy entry's run-time error fires instead of copying an `Arena` (Task 3).

---

### Task 1: Declarations and conformance

**Files:** `syntax/src/token.rs` (`interface` keyword), `syntax/src/ast.rs` (`Item::Interface`, `ifaces` on `StructDecl`/`EnumDecl`), `syntax/src/parser/item.rs` (dispatch; `signature` becomes `pub(super)`), new `syntax/src/parser/iface.rs`, `syntax/src/parser/members.rs` (`: A, B` after the name), `syntax/src/link.rs` + `link/walk.rs` (export, rename, qualify interface names in the lists), `syntax/src/sexpr.rs`; `check/src/collect.rs` (register names in the first pass), new `check/src/iface.rs` (`IfaceInfo`, abstract method `FnInfo`s, conformance), `check/src/tables.rs` (`ifaces`, `iface_by_name`, `StructInfo.ifaces`, `EnumInfo.ifaces`, `FnInfo.is_abstract`), `check/src/collect_ty.rs` (`resolve_type` finds interfaces), `check/src/consts.rs`, `check/src/lib.rs` (skip abstract `FnInfo`s in `check_fn`). Tests: `tests/errors/iface_decl.kes`, parser and checker unit tests.

**Interfaces:** Produces `IfaceId(u32)`, `IfaceInfo { name, methods: Vec<FnId> /* declaration order = table slot order */ }`, `Ty::Iface(IfaceId)` (resolved but not yet usable in bodies; Task 2), `StructInfo.ifaces: Vec<IfaceId>`, `EnumInfo.ifaces: Vec<IfaceId>`, `fn implements(t: &Tables, ty: Ty, i: IfaceId) -> bool`.

- [ ] **Step 1: Failing tests.** `tests/errors/iface_decl.kes`, one line each with the spec's messages: missing method; mismatched signature (wrong param type, missing `var self`, non-pure implementing a `pure` declaration); a pure implementation of a non-pure declaration is accepted (no error line); "`Damageable` is listed twice"; "`Thing` isn't an interface"; "an interface needs at least one method"; a method without `self`; a duplicate method name; an interface name clashing with a struct (existing duplicate-name message). Parser tests for `interface I { pure f(self) -> int \n g(var self, x: int) }`, `pub interface`, `struct G: A, B {`, `enum E: A {`.
- [ ] **Step 2: Run them** (parse errors).
- [ ] **Step 3: Implement.**
- [ ] **Step 4:** Focused tests pass; full suite and clippy; commit: "Declare interfaces and check what implements them".

### Task 2: Interface types in the checker

**Files:** new `check/src/body/iface.rs`; `check/src/body/mod.rs` (`coerce`: struct/enum to `Iface`, and `T` to `I?`), `check/src/body/structs.rs` (`check_method_call` on `Ty::Iface`, `Res::IfaceCall`), `check/src/body/args.rs` (a `var` argument of a concrete implementing type to a `var` interface parameter), `check/src/body/place.rs` (whole assignment to a `var` interface parameter refused), `check/src/body/regions.rs` and `body/moves.rs` (use the coerced type), `check/src/body/collections.rs` (annotated literals coerce each element; the unannotated-mix message), `check/src/body/ops.rs` (`==`), `check/src/tables_ty.rs` (`has_heap`, `is_owning`, `holds_fn` arms), the owning-implementor store refusal, `check/src/body/captures.rs` (7b doesn't look inside). Tests: `tests/errors/iface_use.kes`, checker unit tests.

**Interfaces:** Consumes Task 1. Produces `Res::IfaceCall { iface: IfaceId, slot: u32 }` on a method call through an interface; `Tables::coerce` entries `ExprId -> Ty::Iface(..)` (and `Optional` of one) wherever a concrete value is coerced; a `Tables::iface_borrows: HashSet<ExprId>` of arguments that are directly a concrete value passed to an interface parameter (KIR gives exactly these a borrowed `MakeIface`).

- [ ] **Step 1: Failing tests** (`tests/errors/iface_use.kes`, spec messages): "can't replace `d`: it may be the caller's `Goblin`; change it through its methods"; "interface values can't be compared" (`==`, `has`, map key); "give the list a type: `var foes: list[Damageable] = [...]`"; a non-pure interface method called in a `pure` function and in a `par for` body (existing purity messages naming the method); a `var self` method through a non-`var` place (existing message); "`Sprite` owns a GPU texture, so it can't be stored as a `Drawable`; keep it in a `list[Sprite]`" (use an `Arena`-holding struct; the spec's sentence with "an `Arena`"); a plain-numbers struct coerced to an interface inside `arena { }` and stored outside (existing arena message); a stored one into a parameter outside `main` (existing caller's-arena message); converting an interface value to another interface (existing mismatch message). Checker unit tests: coercion recorded for `let d: Damageable = g`, an argument, `foes.add(g)`, a `return`, a field store, a constructor argument, `I?`; `iface_borrows` contains the direct-argument case only.
- [ ] **Step 2: Run them.**
- [ ] **Step 3: Implement.**
- [ ] **Step 4:** Focused tests pass; full suite and clippy; commit: "Check interface types, coercions and calls".

### Task 3: Boxed interface values, method tables and calls

Every interface value is boxed in this task (correct everywhere, allocating). Task 4 adds borrowing.

**Files:** new `kir/src/lower/iface.rs`, `kir/src/lower/tables.rs`, `codegen/src/func/iface.rs`, `codegen/src/tables.rs`; `kir/src/ir*.rs` (`KTy::Iface`, `ExprKind::MakeIface`, `CallVirtual`, `Module::tables`), every exhaustive `KTy` match and visitor (Global Constraints), `kir/src/lower/helpers.rs` (`fixup` arm emitting `Stmt::CopyIface(place)`), `kir/src/lower/helpers_print.rs`, `codegen/src/lib.rs` (`define_module` defines the tables after function ids), `codegen/src/func/mod.rs` (`Shared::tables`), `runtime/src/panic.rs` (the owning-copy error, if it needs a runtime function). Tests: `tests/programs/ifaces.kes`, `tests/programs/iface_copies.kes`, `tests/programs/iface_owning.kes`, a case in `cli/tests/modules.rs`, KIR and codegen unit tests.

**Interfaces:** Consumes Task 2's `Res::IfaceCall`, `Tables::coerce`. Produces `KTy::Iface` (16 bytes, align 8: data at 0, table at 8); `ExprKind::MakeIface { table: TableIdx, data: IfaceData }` with `IfaceData::Boxed(Box<Expr>)` (Task 4 adds `Borrowed(Place)`); `Stmt::CallVirtual { recv: Place, slot: u32, args: Vec<Arg>, sig: KSig }` and the `ExprKind` twin; `Module::tables: Vec<MethodTable>` with `MethodTable { type_id: u32, copy: FuncIdx, print: FuncIdx, methods: Vec<FuncIdx> }`; `Stmt::CopyIface(Place)`. The table layout in memory: `[type_id: u64, copy: fnptr, print: fnptr, method_0, method_1, ...]`, 8 bytes each.

- [ ] **Step 1: Failing tests.** `tests/programs/ifaces.kes`: a mixed `list[Damageable]` with `var self` calls in a loop, then `hp()` of each; an enum implementing an interface; `Damageable?` with `none` and a value; a map of interface values; `print` of an interface value and of a list of them; a struct with an interface field stored in a list; `filter`/`map`/`sort_by((d) { d.hp() })` over a `list[Damageable]`; a closure capturing an interface value; a `pure` method called in a `par for` over a `list[Damageable]` changing each item. `tests/programs/iface_copies.kes`: copy an interface value, change the copy through a `var self` method, print both; the same for a list of them and a struct holding one; a value stored with `in h.mem { }` into a struct field, then the source arena freed (poison shows any sharing). `tests/programs/iface_owning.kes`: an `Arena`-holding implementor passed to a non-`var` and a `var` interface parameter works; a callee that stores its parameter fails with `// expect-stderr: kestrel: can't copy a \`Pool\` into a \`Holder\` on line N of iface_owning.kes: it owns an \`Arena\`` and `// expect-exit: 101` (use the real names in the test; the spec's sentence shape). `cli/tests/modules.rs`: an interface declared `pub` in `shapes.kes`, implemented in the entry file and in the module, used through `use { Shape from shapes }`.
- [ ] **Step 2: Run them** (KIR panics on `Ty::Iface`).
- [ ] **Step 3: Implement** the lowering (every coercion lowers to `MakeIface{Boxed}`, which allocates the concrete size in the current arena and copies with `owned`), the per-type copy and print functions, the tables as static data with `declare_func_in_data` + `write_function_addr`, `CallVirtual` (load `table[3 + slot]`, call with the data pointer as `self`), and `fixup`/`CopyIface`.
- [ ] **Step 4:** Focused tests pass, JIT and exe; full suite and clippy; commit: "Compile interface values, method tables and calls".

### Task 4: Borrowed interface arguments

**Files:** `kir/src/lower/iface.rs`, `kir/src/lower/call.rs` (never spill a borrowed `MakeIface`), `kir/src/lower/collections.rs` (`owned` turns borrowed into boxed), `kir/src/stack_envs.rs` (also check borrowed `MakeIface`), `kir/src/scalarize.rs`, `kir/src/promote.rs`, `kir/src/lower/lines.rs`. Tests: `tests/programs/iface_args.kes`, a case in a memory test (`cli/tests/iface_memory.rs`), `kir/tests/stack_envs.rs` (or its sibling) extended.

**Interfaces:** Consumes Task 2's `Tables::iface_borrows`, Task 3's `MakeIface`. Produces `IfaceData::Borrowed(Place)`: the data pointer is the place's address; the debug invariant (only as a direct call argument) runs on `lower()`'s output and over every program test.

- [ ] **Step 1: Failing tests.** `tests/programs/iface_args.kes`: `heal(var g)` with `heal(var d: Damageable)` changes `g`; a non-`var` borrowed argument read in the callee; a callee that returns its parameter (the result is a boxed copy: change the original, the result is unchanged); a callee that stores its parameter into `in keep.mem { keep.items.add(d) }` (copied); in a `var self` method, `self.hp = 5` then `show(self)` prints 5 (promote writes back first); a borrowed owning implementor passed through two calls. `cli/tests/iface_memory.rs`: `show(g)` 10 times and 100,000 times in `main` give equal reports.
- [ ] **Step 2: Run them** (the memory test grows).
- [ ] **Step 3: Implement.**
- [ ] **Step 4:** Focused tests pass, the invariant holds over every program test; full suite and clippy; commit: "Pass concrete values to interface parameters without copying".

### Task 5: Measure and document

**Files:** a kernel in `kestrel/bench/kernels` (and its Rust counterpart), `kestrel/bench/kernels/results.md`, `docs/superpowers/specs/2026-10-09-kestrel-syntax-design.md`, `README.md`, the spec's "As built", `docs/superpowers/plans/2026-10-09-core-2a-followups.md`.

- [ ] **Step 1: Kernel:** 1,000,000 `hp()` (or a small `step(var self)`) calls through a `list[Damageable]` of two concrete types, against the same loop over `list[Goblin]` (direct, inlinable) and Rust `Vec<Box<dyn Trait>>`, with `measure.py`'s method. Record numbers, ranges and losses in `results.md`.
- [ ] **Step 2: Docs** in each file's style.
- [ ] **Step 3:** Commit: "Measure interface calls and document interfaces".
