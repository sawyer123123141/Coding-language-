# Interfaces Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add `interface` declarations, `struct X: I` / `enum X: I` conformance, interface types with dynamic dispatch, borrowed and boxed interface values.

**Architecture:** In the checker, `Ty::Iface(IfaceId)` gets abstract method `FnInfo`s and a coercion from implementing types.

In KIR:
- `KTy::Iface` is `{data, table}`.
- `MakeIface { Borrowed(Place) | Boxed(Expr) }` mirrors `MakeFn`'s stack and heap environments and their invariant.
- `CallVirtual` loads the method from the table.

Codegen emits one method table per (type, interface) as static data holding function addresses.

The templates to follow:
- closures' `MakeFn`, `EnvKind`, `check_stack_envs` and the `owned()` conversion (`kir/src/lower/closure.rs`, `kir/src/lower/env.rs`, `kir/src/stack_envs.rs`, `kir/src/lower/collections.rs`);
- `CallIndirect` (`kir/src/lower/fnvalue.rs`, `codegen/src/func/fnvalue.rs`);
- the file-name data records (`codegen/src/globals.rs`).

**Tech Stack:** Rust, Cranelift 0.116.1. Tests are Rust unit tests plus `.kes` program and error tests.

**Spec:** `docs/superpowers/specs/2026-10-19-interfaces-design.md`. Every message and behaviour comes from it, including its "Details settled by the spec review" section. The spec wins any conflict.

## Global Constraints

- **Messages:** plain sentences that say what to do, worded exactly as in the spec.
- **File sizes:** under about 400 lines. New code goes in:
  - `syntax/src/parser/iface.rs`;
  - `check/src/iface.rs` (declarations, conformance) and `check/src/body/iface.rs` (coercion, calls);
  - `kir/src/lower/iface.rs`, `kir/src/lower/tables.rs` (method tables) and `kir/src/lower/iface_copy.rs`;
  - `codegen/src/func/iface.rs` and `codegen/src/tables.rs`.

  Split a file before it passes 400 lines.
- **Every new KIR node** is handled in:
  - `kir/src/visit.rs`, `visit_mut.rs`, `rename.rs` and `inline/descend.rs`;
  - `scalarize.rs` and `promote.rs`: a borrowed `MakeIface` anywhere in a region counts like an `Arg::Ref` of its place's root;
  - `lower/lines.rs` (`may_set_line`) and `lower/sort.rs` (`builds_memory`). `CallVirtual` and `MakeIface{Boxed}` count as setting a line and building memory, conservatively, as `CallIndirect` does;
  - `kir/src/stack_envs.rs`: a stack `MakeFn` or a borrowed `MakeIface` is allowed as a direct argument of `CallVirtual` too;
  - codegen `expr.rs`, `stmt.rs` and `aggregate.rs`.
- **`KTy::Iface` arms:** every exhaustive `KTy` match gets one (`ir.rs`, `layout.rs`, `lower/layout.rs`, `lower/heap.rs`, `lower/mod.rs` `kty`, codegen `func/mod.rs`, `arith.rs`, `print.rs`, and `export.rs` with `Other("an interface")`). `fixup` gets one in `lower/helpers.rs`.
- **Runtime functions** need:
  - an `RtFuncs` field and `decl` in `codegen/src/rt.rs`, a `jit_symbols()` entry, and `#[no_mangle] pub extern "C"`;
  - a runtime callback returns `i8`, never `bool`;
  - a failing one fails through `panic::fail`.
- **Test facts:**
  - **Program tests:** `kestrel/tests/programs/*.kes` with `// expect:`, `// expect-stderr:` and `// expect-exit:` lines, run by `cargo test -p kestrel-cli --test programs programs_` (JIT and exe, poison on).
  - **Error tests:** `kestrel/tests/errors/*.kes` with `// error <line>: <message>` lines.
  - **Multi-file tests** go in `cli/tests/modules.rs`.
  - **Memory tests** follow `cli/tests/text_memory.rs`. `KESTREL_ARENA_REPORT=1` prints `kestrel: program arena N bytes`.
  - No string literals inside `{...}`.
  - A function other than `main` can't store caller-arena memory into a parameter: use `in x.mem { }`.
  - An `Arena` place can't be assigned a new `Arena`.
- **Speed:** while iterating, run focused targets (`kestrel-check`, `kestrel-kir`, `--test programs`, `--test errors`). Run `cargo test --workspace --no-fail-fast` and `cargo clippy --workspace --all-targets -- -D warnings` once before committing.
  - If the user says they are gaming: `CARGO_BUILD_JOBS=2`, `--test-threads=2`, no window, GPU or demo tests, and the full suite is owed.
- **Off-limits:** `kestrel/test.kes`, `voxels.exe`, and the line-ending-only change in `kestrel/check/src/body/enums.rs`. Never use `git add -A`, `git stash` or `git clean`.
- **Commits:** plain-sentence messages ending with `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`. Work on `main`; the controller pushes.

## Review Focus

1. **A borrowed interface value outliving its call:** stored, returned, copied, spilled, or inlined into a `Let` and then stored. It follows the same invariant as stack environments. Tasks 3 and 4 run the debug check over every program test.
2. **Writing a different type into a caller's place through a borrowed `var`:** whole assignment, `var I?`, `var list[I]`, function-type `var` parameters, closures. (Task 2.)
3. **Copies sharing memory:** an interface value copied, a list of them filtered, a struct holding one copied, each changed independently under poison. (Task 4.)
4. **Escape and regions with the coerced type:** a struct of plain numbers coerced to an interface and stored out of `arena { }` is refused. (Task 2.)
5. **Owning implementors:** borrowing is fine, a non-place one is refused, storing is refused at compile time, and the copy entry's run-time error fires instead of copying an `Arena`. (Tasks 2 and 4.)

---

### Task 1: Declarations and conformance

**Files:**
- Syntax:
  - `syntax/src/token.rs`: the `interface` keyword;
  - `syntax/src/ast.rs`: `Item::Interface`, and `ifaces` on `StructDecl` and `EnumDecl`;
  - `syntax/src/parser/item.rs`: dispatch, and `signature` becomes `pub(super)`;
  - new `syntax/src/parser/iface.rs`;
  - `syntax/src/parser/members.rs`: `: A, B` after the name;
  - `syntax/src/link.rs` and `link/walk.rs`: export, rename, and qualify interface names in the lists;
  - `syntax/src/sexpr.rs`.
- Checker:
  - `check/src/collect.rs`: register names in the first pass;
  - new `check/src/iface.rs`: `IfaceInfo`, abstract method `FnInfo`s, conformance;
  - `check/src/tables.rs`: `ifaces`, `iface_by_name`, `StructInfo.ifaces`, `EnumInfo.ifaces`, `FnInfo.is_abstract`, and an arm in `FnInfo::decl` (currently `unreachable!` for other items, `tables.rs:164-171`);
  - `check/src/collect_ty.rs`: `resolve_type` finds interfaces;
  - `check/src/body/consts.rs`;
  - `check/src/body/mod.rs`: `check_fn` (`:104`) skips abstract `FnInfo`s.
- KIR: `kir/src/lower/mod.rs` (`:43-45`, `:120`, `:149`) and `lower/lines.rs:24`. Abstract `FnInfo`s are lowered as stub functions that trap, so `FuncIdx == FnId` stays true and every program that declares an interface still compiles. The Godot export skips them.
- Tests: `tests/errors/iface_decl.kes`, `tests/programs/iface_decl.kes`, parser and checker unit tests.

**Interfaces:**
- Produces:
  - `IfaceId(u32)`;
  - `IfaceInfo { name, methods: Vec<FnId> }`, with methods in declaration order, which is the table slot order;
  - `Ty::Iface(IfaceId)`, resolved here but not yet usable in bodies (that is Task 2);
  - `StructInfo.ifaces` and `EnumInfo.ifaces`;
  - `fn implements(t: &Tables, ty: Ty, i: IfaceId) -> bool`.

- [ ] **Step 1: Write the failing tests.**
  - `tests/errors/iface_decl.kes`, one line each, with the spec's messages:
    - a missing method;
    - a mismatched signature: wrong parameter type, missing `var self`, a different `var` on a non-`self` parameter, a non-pure implementation of a `pure` declaration;
    - "`Damageable` is listed twice";
    - "`Thing` isn't an interface";
    - "an interface needs at least one method";
    - "`area` in `Shape` needs `self` or `var self` first";
    - "`Shape` declares `area` twice";
    - an interface name that clashes with a struct (the existing duplicate-name message).
  - `tests/programs/iface_decl.kes`: a program that declares an interface and two implementors, including a pure implementation of a non-pure declaration. It calls their methods directly and prints, and it must compile and run.
  - Parser tests: `interface I { pure f(self) -> int \n g(var self, x: int) }`, `pub interface`, `struct G: A, B {` and `enum E: A {`.
- [ ] **Step 2: Run them.** Expected: parse errors.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Verify and commit.** The focused tests pass; then the full suite and clippy. Commit: "Declare interfaces and check what implements them".

### Task 2: Interface types in the checker

**Files:**
- New `check/src/body/iface.rs`.
- `check/src/body/mod.rs`, in `coerce`:
  - a struct or enum to `Iface`;
  - `T` to `I?`;
  - no container coercion.
- `check/src/body/structs.rs`: `check_method_call` on `Ty::Iface` goes through the existing receiver path (`receiver_root`, `check_par_write`, `map_loop_inside`, `check_changed_later`) and records `Res::IfaceCall`.
- Every `Res::Fn` method-call site learns `Res::IfaceCall`: `check/src/body/nested.rs:29` (`changes_in`), `check/src/body/call.rs:94` (the unused-result warning), and the others found by grep.
- `check/src/body/args.rs` and `check/src/body/fnvalue.rs` (`check_value_args`):
  - a `var` argument of a concrete implementing type goes only to exactly `var I`;
  - `iface_borrows` is recorded here.
- `check/src/body/place.rs`: whole assignment to a `var` interface parameter is refused, including closure and function-type parameters.
- `check/src/body/regions.rs` and `body/moves.rs`: use the coerced type.
- `check/src/body/collections.rs`:
  - annotated literals coerce each element;
  - the message for an unannotated mix;
  - `array[T](n)` is refused when `T` holds an interface.
- `check/src/body/ops.rs` and `tables_ty.rs`: a new `holds_uncomparable` predicate (function or interface) for `==`. `holds_fn` stays for rule 7b and doesn't look inside interfaces. `has_heap` and `is_owning` get arms.
- The owning-implementor refusals: storing, and a non-place argument.
- `check/src/natives.rs`: natives with interface parameters or results are refused.
- Tests: `tests/errors/iface_use.kes` and checker unit tests.

**Interfaces:**
- Consumes Task 1.
- Produces:
  - `Res::IfaceCall { iface: IfaceId, slot: u32 }`;
  - `Tables::coerce` entries `ExprId -> Ty::Iface(..)` (or an `Optional` of one) wherever a concrete value is coerced;
  - `Tables::iface_borrows: HashSet<ExprId>`: arguments that are directly a concrete **place** passed to an interface parameter, `var` or not. KIR borrows exactly these, and boxes every other coercion.

- [ ] **Step 1: Write the failing tests.** `tests/errors/iface_use.kes`, with the spec's messages:
  - "can't replace `d`: it may be the caller's `Goblin`; change it through its methods", for a function, a closure and a function type;
  - `var g` passed to `var o: Damageable?` and to `var list[Damageable]` (the existing mismatch message);
  - `list[Goblin]` passed as `list[Damageable]` (mismatch);
  - "interface values can't be compared", for `==` on two interface values and on a struct holding one;
  - `has` on a `list[Damageable]` (the existing message);
  - "give the list a type: `var foes: list[Damageable] = [...]`";
  - "`array[Damageable](n)` would start with empty items; build it with a literal or `add`";
  - a non-pure interface method called in a `pure` function and in a `par for` body (the existing purity messages);
  - a `var self` method called through a non-`var` place;
  - the owning store refusal (the spec's sentence, with "an `Arena`");
  - "`Pool()` owns an `Arena`; store it in a `let` first, then pass it";
  - a struct of plain numbers coerced inside `arena { }` and stored outside (the existing arena message);
  - storing one into a parameter outside `main`;
  - converting one interface value to another interface (mismatch);
  - "native functions can't take interface values".

  Checker unit tests:
  - coercion is recorded for `let d: Damageable = g`, an argument, `foes.add(g)`, a `return`, a field store, a constructor argument, and `I?`;
  - `iface_borrows` contains the place-argument cases, `var` included, and not `show(Goblin())`;
  - `run(() { d.hp() }, d.hit(2))`, where `hit` is a `var self` interface method, is refused by the closure exclusivity rule.
- [ ] **Step 2: Run them.**
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Verify and commit.** The focused tests pass; then the full suite and clippy. Commit: "Check interface types, coercions and calls".

### Task 3: Interface values, method tables and calls

**Files:**
- New `kir/src/lower/iface.rs`, `kir/src/lower/tables.rs`, `codegen/src/func/iface.rs` and `codegen/src/tables.rs`.
- `kir/src/ir*.rs`: `KTy::Iface`, `ExprKind::MakeIface`, `CallVirtual`, `Module::tables`.
- Every exhaustive `KTy` match and every visitor (see Global Constraints), and `kir/src/stack_envs.rs`.
- Method-call lowering: `kir/src/lower/expr.rs:102-130`, `lower/stmt.rs:122` and `lower/call.rs`. A `var` interface argument is `Arg::Value(MakeIface{Borrowed(place)})`, never `Arg::Ref(place)`; `inline.rs:135` already handles an `Arg::Value` going to a by-pointer parameter. The same applies in `lower/fnvalue.rs` and to `CallVirtual` arguments.
- The `T` to `I?` branch in `kir/src/lower/expr.rs:14-18`.
- `kir/src/lower/helpers_print.rs`.
- `codegen/src/lib.rs`: `define_module` defines the tables after the function ids exist.
- `codegen/src/func/mod.rs`: `Shared::tables`.
- Tests: `tests/programs/ifaces.kes`, `tests/programs/iface_args.kes`, a case in `cli/tests/modules.rs`, `cli/tests/iface_memory.rs`, and KIR and codegen unit tests.

**Interfaces:**
- Consumes Task 2's `Res::IfaceCall`, `Tables::coerce` and `Tables::iface_borrows`.
- Produces:
  - `KTy::Iface`: 16 bytes, align 8, with data at 0 and the table at 8;
  - `ExprKind::MakeIface { table: TableIdx, data: IfaceData }`, where `IfaceData::Borrowed(Place)` (the data pointer is the place's address) or `IfaceData::Boxed(Box<Expr>)` (the concrete size is allocated in the current arena and copied with `owned`);
  - `owned()` turns a borrowed `MakeIface` into a boxed one, and a borrowed one is never spilled;
  - `Stmt::CallVirtual { recv: Place, slot: u32, args: Vec<Arg>, sig: KSig }` and its `ExprKind` twin. `sig.params` excludes `self`. Codegen passes the result pointer (if any), then the data pointer, then the arguments, which is the methods' real convention, so the table holds the methods themselves;
  - `Module::tables: Vec<MethodTable>`, with `MethodTable { type_id: u32, copy: FuncIdx, print: FuncIdx, methods: Vec<FuncIdx> }`;
  - the table's memory layout: `[type_id: u64, copy, print, method_0, method_1, ...]`, 8 bytes each. It is static data written with `declare_func_in_data` and `write_function_addr`, which both the JIT and object backends relocate.

  The copy entry is a stub in this task (Task 4 builds copies); a test that copies must not exist yet.

- [ ] **Step 1: Write the failing tests.**
  - `tests/programs/ifaces.kes`:
    - a mixed `list[Damageable]` with `var self` calls in a loop, then the `hp()` of each;
    - an enum implementing an interface, with an enum `self`;
    - a method returning a struct through the table (sret);
    - `Damageable?` with `none` and with a value;
    - `print` of an interface value and of a list of them;
    - a `pure` method called in a `par for` over a `list[Damageable]`, changing each item;
    - a closure literal passed to an interface method.
  - `tests/programs/iface_args.kes`:
    - `heal(var g)` with `heal(var d: Damageable)` changes `g`;
    - a non-`var` borrowed argument, read in the callee;
    - labelled and reordered arguments with a borrowed argument, both `var` and not;
    - in a `var self` method, `self.hp = 5` then `show(self)` prints 5 (promote writes back first);
    - an owning implementor passed borrowed through two calls, `var` and not;
    - `show(Goblin())` (boxed, not a place).
  - `cli/tests/modules.rs`: an interface declared `pub` in `shapes.kes`, implemented in the entry file and in the module, and used through `use { Shape from shapes }`.
  - `cli/tests/iface_memory.rs`: `show(g)` called 10 times and 100,000 times in `main` gives equal reports.
- [ ] **Step 2: Run them.** Expected: KIR panics on `Ty::Iface`.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Verify and commit.** The focused tests pass in JIT and exe, and the stack invariant holds over every program test. Then the full suite and clippy. Commit: "Compile interface values, method tables and calls".

### Task 4: Copies of interface values

**Files:**
- New `kir/src/lower/iface_copy.rs`.
- `kir/src/lower/helpers.rs`: a `fixup` arm emitting `Stmt::CopyIface(place)`.
- `kir/src/lower/tables.rs`: the real copy entries.
- `codegen/src/func/iface.rs`: `CopyIface` calls `table[1]` with the data pointer and stores the result.
- `runtime/src/panic.rs`: add a runtime function for the owning error if one is needed.
- Tests: `tests/programs/iface_copies.kes`, `tests/programs/iface_lists.kes`, `tests/programs/iface_owning.kes`.

**Interfaces:**
- Consumes Task 3.
- Produces a copy entry per (type, interface): a KIR function `(data: I64) -> I64`. It allocates the concrete size in the current arena (a new sized-allocation expression, since `EnvDup` reads its size from a header), copies the bytes, binds the copy as a `RefParam` of the concrete type (reusing `BindEnv` or a sibling) and `fixup`s it. For an owning type it fails instead, with the spec's message, through `panic::fail`.

- [ ] **Step 1: Write the failing tests.**
  - `tests/programs/iface_copies.kes`:
    - copy an interface value, change the copy through a `var self` method, print both;
    - the same for a struct holding one, and for a list of them;
    - a value stored with `in h.mem { }` into a struct field, after which the source arena is freed (poison shows any sharing);
    - a callee that returns its borrowed parameter: change the original, and the result is unchanged;
    - a callee that stores its parameter with `in keep.mem { keep.items.add(d) }`;
    - an interface-typed constant (`let DEFAULT: Brain = Dumb()`), and a field default holding an interface value.
  - `tests/programs/iface_lists.kes`:
    - `filter`, `map` and `sort_by((d) { d.hp() })` over a `list[Damageable]`;
    - a map of interface values;
    - a closure capturing an interface value, called after the source changes.
  - `tests/programs/iface_owning.kes`: a callee that stores its borrowed owning parameter fails with the spec's message (`// expect-stderr:` with the real type names, and `// expect-exit: 101`).
- [ ] **Step 2: Run them.**
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Verify and commit.** The focused tests pass in JIT and exe; then the full suite and clippy. Commit: "Copy interface values through their method tables".

### Task 5: Measure and document

**Files:**
- A kernel in `kestrel/bench/kernels`, with its Rust counterpart, and `kestrel/bench/kernels/results.md`.
- `docs/superpowers/specs/2026-10-09-kestrel-syntax-design.md`, `README.md`, the spec's "As built", and `docs/superpowers/plans/2026-10-09-core-2a-followups.md`.

- [ ] **Step 1: The kernel.** Make 1,000,000 calls of `hp()` (or a small `step(var self)`) through a `list[Damageable]` of two concrete types. Compare against the same loop over `list[Goblin]` (direct and inlinable) and against Rust's `Vec<Box<dyn Trait>>`, using `measure.py`'s method. Record the numbers, ranges and losses in `results.md`.
- [ ] **Step 2: Docs,** each in its file's style.
- [ ] **Step 3: Commit:** "Measure interface calls and document interfaces".
