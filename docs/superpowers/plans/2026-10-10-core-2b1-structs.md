# Core Rebuild 2b-i: Structs and Methods

**Goal:** Kestrel programs can declare structs with fields, defaults and methods (`self` / `var self`), construct them (`Point(1, 2)`, `Point(y: 2, x: 1)`), read and write fields (`p.x`, `p.x += 1`, nested `a.b.c`), pass them to functions (read-only by pointer, `var` by pointer), return them, print them, and run all of it via JIT and exe. Labelled arguments evaluate in source order.

**Spec:** `docs/superpowers/specs/2026-10-09-kestrel-core-rebuild-design.md` (§2 layout + value semantics), `docs/superpowers/specs/2026-10-09-kestrel-syntax-design.md` (structs, methods, construction).

**Execution note:** written for and executed by the controller directly (subagents rate-limited at the time), so tasks list interfaces, design decisions and tests rather than full code. Each task is TDD and ends with a commit; a review pass runs afterwards.

## Design decisions

- **Scope of struct fields in 2b-i:** scalars and other structs. Arrays, lists, strings and arenas come later. A struct that contains itself is an error.
- **Field defaults** must be constant expressions, under the same rule as top-level constants. (Defaults like `Arena()` arrive with 2c.)
- **Value semantics:** `var b = a` copies. Read-only struct parameters are passed by pointer (no copy); `var` parameters and `var self` are passed by pointer to the caller's place.
- **Return values:** struct results go through a caller-provided return slot (first pointer parameter).
- **No struct `==`** yet (it's an error, since the operands aren't numbers).
- **`print(p)`** prints `Point(x: 1, y: 2)`, with nested structs printed recursively.
- **`pure` methods** take `self`. A `var self` method is never pure, so pure code can't call it.
- **Exclusivity** extends from bare variables to place roots: `f(var p.x, p)` is an error. Disjoint fields of one root are conservatively rejected too.
- **Types:** `Ty` stays a `Copy` enum and gains `Struct(StructId)`. Type names come from `Tables::ty_name(Ty) -> String`.
- **KIR additions:**
  - `KTy::Struct(StructIdx)`, plus `Module::structs: Vec<StructLayout { name, fields: Vec<FieldLayout { name, ty, offset }>, size, align }>`.
  - `Place { root: LocalIdx, fields: Vec<u32> }`.
  - `ExprKind::Load(Place)` replaces `Local(LocalIdx)` and covers scalar and aggregate places.
  - `Stmt::Assign(Place, Expr)`.
  - `ExprKind::StructLit(StructIdx, Vec<Expr>)`, with fields in declaration order and defaults already filled in.
  - `ExprKind::FieldOf(Box<Expr>, u32)` for fields of non-place values.
  - `ExprKind::Seq(Vec<Stmt>, Box<Expr>)`, which runs the statements and then yields the value. It's used for source-order argument temporaries.
  - `Arg::Ref(Place)`.
  - Lowering can add synthetic locals.
- **Codegen:**
  - Aggregate locals live in stack slots. Aggregate parameters and `RefParam` locals hold pointers.
  - A place's address is the root address plus constant field offsets.
  - Aggregate expressions lower *into* a destination address (`lower_into`).
  - Copying is field by field: scalar loads and stores generated from the layout, with no memcpy libcall.
  - Assigning an aggregate goes through a temporary, so `p = Point(p.y, p.x)` is correct.
  - `func/mod.rs` gets split first: print/string helpers move to `func/print.rs`, and places and aggregates go in `func/place.rs` and `func/aggregate.rs`.

## Tasks

1. **Parser.**
   - Covers `struct Name { field: Type [= default] ... methods }`, `self` / `var self` as the first parameter, the postfix `.field` and `.method(args)`, and `self` as an expression.
   - The AST gains `Item::Struct(StructDecl { name, fields: Vec<FieldDecl { name, ty, default, span }>, methods: Vec<FnDecl>, ... })`, `ExprKind::Field { base, name, name_span }` and `ExprKind::MethodCall { receiver, name, name_span, args }`. `Param` gains `is_self`.
   - Tests: s-expressions for field chains, method calls and struct declarations, plus errors for `self` outside the first position.
2. **Checker: struct types.**
   - Collect structs in two phases: names first, then field types. Report a struct that contains itself, duplicate fields, and defaults that aren't constant.
   - Constructor calls bind arguments to fields: positional first, then labelled, with defaults filling skipped fields. Results go in `Res::Construct(StructId)` and `ctor_args: HashMap<ExprId, Vec<FieldArg>>`, where `FieldArg` is `Arg(usize)` or `Default`.
   - Type field reads, and assignment to a place: the root must be changeable, the path must be fields, and compound operators need numeric fields.
   - `print` accepts structs.
   - Tests: an ok program, plus one error test per rule.
3. **Checker: methods.**
   - Methods are functions with `FnInfo.owner: Option<StructId>` and a self parameter. Calls are resolved by receiver type.
   - A `var self` call needs a changeable receiver place. Receivers are exempt from the call-site `var` marker.
   - Purity: pure methods can't take `var self`, and pure code can't call `var self` methods.
   - Exclusivity uses place roots.
   - Tests: an ok program, plus errors (immutable receiver, unknown method, pure calling `var self`, `f(var p.x, p)`).
4. **KIR: aggregates.**
   - Implement the layout computation and all the new IR above, including lowering for struct literals (with defaults), field places, method calls (the self argument becomes `Arg::Ref(place)`, or a temporary for an rvalue receiver), and struct-returning calls.
   - Labelled arguments evaluate in source order through `Seq` temporaries whenever the order differs from the parameters.
   - Tests: KIR-shape tests for each.
5. **Codegen: aggregates.**
   - Split `func/mod.rs` first.
   - Then add aggregate locals, parameters (pointers), returns (return slot), place loads and stores, `lower_into`, field-by-field copies, struct literals, `FieldOf`, `Seq`, method calls, and struct printing.
   - Tests: JIT tests (struct round trips, nested fields, `var self` mutation, returns, copies being independent), then un-ignore `labelled_arguments_evaluate_in_source_order`.
6. **End to end and benchmark.**
   - Program tests `structs.kes`, `methods.kes`, `struct_copies.kes`, plus error tests for the new diagnostics.
   - A particle-update benchmark (structs of floats, method calls in a hot loop) against Rust, added to `kestrel/bench/`.
