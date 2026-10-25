# Generics: design (2026-10-24)

**Source:** the proposal `docs/superpowers/plans/2026-10-24-generics-proposal.md`, approved 2026-10-24: square brackets (pick 1), types worked out from values and the expected type (pick 2), what `T` may do comes only from its rules (pick 3), several rules with `+` (pick 4), generic enums this round (pick 5); no `number` rule (A), the three built-in rules `equatable`, `ordered`, `key` (B), capitalised placeholder names that can't hide a type (C); Claude's rulings R1-R8. This spec doesn't reopen any of them. It settles what the proposal left to it ("For the spec to settle") and everything a plan needs: the representation, the parsing, checking generic code once, working out types, the collection of copies, lowering per copy, names, modules, tests and tasks.

Rulings in this document that aren't the user's picks are Claude's (G1-G27 at the end), each with what it costs if wrong. Everything here was designed against the code as of bad78c5 (read only; nothing was built or run).

## For the user: what this spec decides that you would see

Six things go beyond what the proposal showed. They are built as written unless you say otherwise; each is in one place.

1. **`<` on strings and bools, for everyone (G5).** The proposal says `T: ordered` gives `<`, `<=`, `>`, `>=` and that "today" those work on numbers, `bool` and `string`. They don't: today `<` takes numbers only (`sort()` is what takes strings and bools). A copy of `[T: ordered]` code made for `string` has to compare strings, so this round makes `<`, `<=`, `>`, `>=` work on two strings (by character, the order `sort()` uses) and two bools (`false < true`) in all code. *Alternative:* `ordered` gives only `sort()` and `sort_by` keys, and `<` on a `T` is refused.
2. **The expected type only fills what the values leave open (G7).** `let x: float = first(ints)` works (`T` is `int`, and the `int` result widens), but `let s: Stack[float] = Stack(items: [1, 2])` is an error (`T` is `int` from the list, which must match exactly); write `Stack[float](items: [1, 2])` or `[1.0, 2.0]`. Closures and `none` are worked out after the expected type, so `let ys: list[float] = convert(xs, (x) { x })` gives `float`.
3. **A type that holds a function anywhere can't be `T` (G6),** a `list[(int) -> int]` included (the proposal named function types and structs or enums that hold one).
4. **Placeholder names must start with a capital letter (G3):** "a placeholder's name starts with a capital letter, like `T` or `Item`".
5. **`ordered` and `key` include `equatable` (G4):** `[T: ordered]` may use `==`, since every type it allows can be compared.
6. **Endless copies are an error where they are written (G10),** even if nothing uses the generic yet, like a struct that contains itself. A few that go through a rule's method can only be found when copies are made, and stop with an error there.

Nothing in the proposal turned out infeasible.

## The language

### Declaring generics

- **Placeholders** follow the name of a struct, an enum or a function, in square brackets, separated by commas: `struct Stack[T]`, `enum Result[T, E]`, `best[T](...)`, `pure best[T](...)`, `pub best[T](...)`. A struct's or enum's interface list comes after them: `struct Stack[T]: Named { ... }`.
- **Rules** follow a placeholder after `:`, joined with `+`: `[T: Damageable + Named, K: key]`. A rule is an interface's name or one of the three built-in rules `equatable`, `ordered`, `key`. The three are words only in this position: `key` stays usable as a variable or function name. An interface can't be named `equatable`, `ordered` or `key`.
- **Names:** a placeholder's name starts with a capital letter (G3). It can't be the name of a type that exists (a struct, enum or interface anywhere in the program, or a built-in type): Kestrel never lets one name hide another (question C). Functions and constants with the same name are fine, since a placeholder only appears where a type goes (G16). Within one list each name and each rule appears once.
- **Methods use their struct's or enum's placeholders** and have none of their own (R8). Interfaces, natives and `main` have no placeholders.
- **Inside its own declaration, a generic type's bare name means itself with its own placeholders:** in `struct Stack[T]`, `Stack` is `Stack[T]`, so `clone(self) -> Stack` works and `self` needs nothing written (G2). Everywhere else a generic type is written with its types.
- **Two interface rules whose methods share a name** are fine when the methods have the same signature (a call means either); when the signatures differ, no type can satisfy both: "no type can be both `A` and `B`: their `name` methods differ" (G9).

### Using generics

- **In types:** `Stack[int]`, `Pair[string, vec2]`, `list[Stack[int]]`, `map[string, Result[Level, string]]`, `Stack[T]` inside other generic code. Types from another module are imported by name (`use { Stack from geom }`), as for any type: a type can't be written `geom.Stack` today.
- **Brackets after a name in an expression** (pick 1) mean type arguments when the name is a generic struct, enum or function, and an index otherwise:

  ```
  best[Enemy](foes, score)        // types, then a call
  Stack[int]()                    // types, then a construction
  Result[int, string].Ok(value: 3)
  Maybe[int].Nothing
  geom.best[Enemy](foes, score)   // the same through a module
  geom.Stack[int]()
  let f = best[Enemy]             // a function value
  handlers[i](3)                  // an index: `handlers` is a local
  self.handlers[i](3)             // an index: a field
  TABLE[i]                        // an index: a constant
  ```

  What's inside the brackets may be any types, including ones that aren't expressions (`best[list[int]]`, `Box[(int) -> int]` is refused by R3 but parses, `Stack[int?]` likewise). `Stack[int]` on its own as a value is "`Stack[int]` is a type; build one with `Stack[int](...)`".
- **Calls and constructions** work out their types (pick 2, below); writing them is optional: `best(foes, score)`, `Stack()` where the expected type says which, `Pair(a: 1, b: "x")`, `Result.Ok(value: lvl)` in a function returning `Result[Level, string]`.
- **Written types give all of them** (G8): `best[Enemy](...)`, never some. "`Result` takes 2 types, found 1".
- **Methods** are called on a value as usual (`ints.push(3)`); the value's type fixes the placeholders.
- **A generic function is a value once its types are fixed** (R8): `let f = best[Enemy]` and `let RULE = best[int]` (a constant) work; `let f = best` is "`best` is generic; give its types to use it as a value, like `best[int]`".
- **`match`** on a generic enum names variants as for any enum (`Ok r { play(r.value) }`); `r.value` has the copy's type.

### What `T` can do inside generic code

Generic code is checked once, with `T` unknown (R1). A `T` value can be stored, copied, passed, returned, printed (`print(x)`, `"{x}"`), held in a `list[T]`, `[T]` literal, `T?`, map value or another generic type, captured by a closure, and passed where a `T?` is expected. Everything else comes from its rules:

| Written | Needs | Otherwise |
|---|---|---|
| `x.hp()` | an interface rule with method `hp` | "`T` has no method `hp`: its rules (`Named`) don't have one", or "`T` has no rules, so it has no methods; add one, like `[T: Damageable]`" |
| `x` where a `Damageable` is expected | the rule `Damageable` | "expected Damageable, found T" |
| `x == y`, `x != y`, `xs.has(x)`, `xs.find(x)` | `equatable` (or `ordered`, `key`) | "`==` on `T` needs the rule `equatable`: write `[T: equatable]`"; "`has` needs `T` to be `equatable`: write `[T: equatable]`" |
| `x < y` (and `<=`, `>`, `>=`), `xs.sort()`, `T` as a `sort_by` key | `ordered` | "`<` on `T` needs the rule `ordered`: write `[T: ordered]`"; "`sort` needs `T` to be `ordered`: write `[T: ordered]`" |
| `map[K, V]` | `K: key` | "`K` can't be a map key without the rule `key`: write `[K: key]`" |

- **Fields of `T`:** "`x` is a `T`, which can be any type, so it has no field `hp`", help "give `T` a rule with a method instead, like `[T: HasHp]`".
- **Arithmetic and other operators** (no `number` rule this round, question A): "`+` doesn't work on `T`, which can be any type; take a function that does it, like `add: (T, T) -> T`". Conversions (`float(x)`) keep today's "can't convert T to float".
- **`T` names a type, not a value:** `T(...)` is "`T` is a placeholder, so it can't be built here; take a value of it as a parameter instead"; `T` alone is "`T` is a type, not a value".
- **Zero-filled arrays** of anything holding a `T` (`array[T](n)`, `array[Box[T]](n)`): "`array[T](n)` fills its items with zeros, which a `T` may not have; build a `list[T]` with `add` instead" (R4). `list[T]()`, `map[K, V]()` and `[a, b]` are fine.
- **`pure` code and `par for`:** a call through an interface rule follows the interface's method: it must be `pure` there (R4), with the existing messages naming the interface's method (`Damageable.hp`).
- **Closures** in generic code may take and return `T` (`(x: T) { ... }`, or untyped where a function type is expected) and capture `T` values; `T` is never owning, so every capture rule is the plain one.
- **Field defaults and labelled construction:** a generic struct's defaults are checked once (`items: list[T] = []`), built for each copy; `Stack(items: xs)` and `Pair(b: 2, a: 1)` bind as for any struct. A field of type `T` can't have a default except `none` for `T?` (nothing else has type `T`).
- **The strict answers (R4).** Every question the checker asks of a type gets, for `T` and any type containing it, the answer that refuses more code, so the decisions it records (views, moves, conversions, where values are passed by pointer) are right for every copy:

| Question | Answer for `T` |
|---|---|
| passed by pointer (`Ty::is_aggregate`) | yes: the exclusivity and "changed by a later argument" rules apply as for a struct |
| holds heap memory (`has_heap`) | yes: copies may allocate, arena escape rules as for a `string`, kept-part checks |
| owning (`is_owning`, `holds_arena`) | no (R3); a generic type's owning-ness is its declaration's |
| holds a function (`holds_fn`) | no (R3, G6) |
| holds an interface (`holds_iface`) | yes |
| breaks a zero fill (`zero_fill_breaks`) | yes |
| can't be compared (`holds_uncomparable`) | yes, unless it has `equatable` |
| sortable, a `sort_by` key | only with `ordered` |
| a map key (`map_ty`) | only with `key` |
| a vector, a number, an integer, a float | no |
| converts to (`coerces_to`) | only itself, `T?`, and an interface it has as a rule |
| contained in place (self-containment) | nothing: `T` is a leaf |
| a `let ... else` view is worth it | yes (a view) |
| a default allocates | as for any type with heap memory |
| a map value that owns | no |

### Which types can be `T`

Checked wherever a placeholder gets a type: a written type (`Stack[X]`), written type arguments, and worked-out ones (R3, R7):

- not owning: "`Sprite` owns a GPU object, so it can't be `T` yet" ("owns an `Arena`" for an arena);
- not an optional: "`int?` is an optional, so it can't be `T` yet; hold it in a struct field instead";
- not a function type and not a type holding one anywhere (G6): "`(int) -> int` is a function type, so it can't be `T` yet", "`Button` holds a function value, so it can't be `T` yet";
- meets every rule: "`Rock` isn't `Damageable`, so it can't be `T` in `total_hp`"; "`Enemy` isn't `equatable` (numbers, bools, strings and vectors are), so it can't be `T` in `count`"; "`vec2` isn't `ordered` (numbers, bools and strings are), so ..."; "`float` isn't a `key` (integers, bools and strings are), so ...". An interface type meets its own rule only (`Damageable` can be `T` in `total_hp`; that copy calls through the interface). A placeholder of the surrounding generic meets a rule it carries (`ordered` and `key` carry `equatable`), else the message adds "; add the rule: `[U: Damageable]`".
- The sets match what Kestrel accepts today: `equatable` is `int i64 u8 float f64 bool string vec2 vec3 vec4 ivec2 ivec3 ivec4`; `ordered` is the numbers, `bool` and `string`; `key` is `int i64 u8 bool string`.
- Interface types, lists, arrays, maps and other generic types are allowed (`Stack[Damageable]`, `Stack[list[int]]`, `Stack[Stack[int]]`).

### Working out types (pick 2)

For a call of a generic function, a construction of a generic struct and a variant of a generic enum, when no types are written. The algorithm, in order (G7):

1. **Values.** Each argument, in source order, except those deferred to step 3, is checked with the parameter's type as its hint when that type is already known (placeholders bound so far filled in), otherwise with no hint. Its type is matched against the parameter's type: a placeholder at the top meets the value's type; inside `list[T]`, `[T]`, `T?` (when the value is optional), `map`, a function type or another generic type, the parts meet. A value given for `T?` that isn't optional meets `T`.
   - **Numbers widen loosely:** a placeholder met by an `int` and then a `float` (or `i64`) becomes the common type, as mixed numbers do, and the earlier arguments are converted. **Except exactly:** a placeholder met through a `var` argument, inside a type (`list[T]`, `T?`, a function type ...), or by a closure's result must match exactly, and a loose meeting can't widen it.
2. **The expected type** (a `let` annotation, a parameter, a field, a `return`, an assignment) fills only the placeholders the values left open; it never changes one they decided.
3. **Deferred arguments,** in source order: closure literals with a parameter whose type isn't written, `none`, `[]` and `{}`. Each is checked with the parameter's type filled in. A closure's parameter types must be known by now; its result type, if the parameter's result is still open, comes from its body and decides that placeholder exactly (`make(() { 3 })` gives `int`). An empty `[]` or `none` decides nothing.
4. **Every placeholder must be decided,** then checked against R3 and its rules, at the argument that decided it (else at the call). The arguments are then checked against the filled-in parameter types with the usual conversions and messages ("expected Stack[float], found Stack[int]").

Messages:
- undecided: "can't tell what `T` is in `best`; write it, like `best[int](...)`"; for a struct "... like `Stack[int]()`", a variant "... like `Maybe[int].Nothing`" or "`Result[int, string].Ok(...)`". The example fills in the placeholders already decided and writes `int` for the rest (G26).
- conflict: "`T` in `pick` can't be both `int` (from `a`) and `string` (from `b`)"; exact: "`T` in `push_all` can't be both `int` (from `xs`, which must match exactly) and `float` (from `v`)", help "write the type, like `push_all[float](...)`".

A placeholder no parameter mentions can only be written (`make_empty[int]()`); that is allowed (G18).

### Endless copies

Each copy is compiled code, so a generic can't need endless copies (R2). Two errors, both at the declaration, whether or not anything uses it (G10):
- "`Tree` would need endless copies: `Tree[T]` holds a `Tree[list[T]]`, which holds a `Tree[list[list[T]]]`, and so on", help "use the same placeholders (`Tree[T]`), or types that don't grow".
- "`walk` would need endless copies: `walk[T]` calls `walk[list[T]]`, which calls `walk[list[list[T]]]`, and so on".

A type that holds itself through a generic is today's error: "struct `Bad` contains itself" for `struct Bad { w: Wrap[Bad] }` when `Wrap` holds its `T` in place, and "enum `L` contains itself" for `enum L[T] { Cons { tail: L[T]? } }`, with today's help. A cycle that only shows when copies are made (through a rule's method) stops there: "making copies of `walk` doesn't end: `walk[list[list[int]]]` needs `walk[list[list[list[int]]]]`, and so on (stopped after 32 levels)".

### Copies, names, contracts and the report

- **One copy per generic and set of types in the whole program** (R6), whichever modules use it. Copies are ordinary functions, structs and enums to everything after the checker: range analysis, the inliner, contracts and codegen.
- **Names:** a copy of a struct or enum is `Stack[int]`, of a function `best[Enemy]`, of a method `Stack[int].push`; a module's types keep their module name in messages (`geom.Stack[int]`), as types do today. `print` and `"{x}"` show `Stack[int](items: [3])`, with each type argument as `print` names types (no module name) (G17).
- **`@prove` and `kestrel check --report` work per copy** (R5): `@prove(no_alloc)` on `Stack.push` is checked on every copy. An error names the copy and the line that made it: "`@prove(no_alloc)` on `Stack[string].push` doesn't hold: it allocates on line 5: copying a `string` (made by `Stack[string]` on line 12)", in today's wording plus the clause in parentheses. The line is the first place in non-generic code that led to the copy (G21).
- **A generic with no copies** has nothing to check: its contract holds vacuously, and the report lists it as "`best`: not checked, no copies" (G15).
- **Godot:** generic functions and their copies aren't exported (`functions()` doesn't list them); a non-generic wrapper is the way.

### Modules

`pub struct Stack[T]`, `pub enum Result[T, E]` and `pub best[T]` are imported like any `pub` item (`use { Stack, best from geom }`, or `use geom` and `geom.best(...)`, `geom.best[Enemy](...)`, `geom.Stack[int]()`, `geom.Result[int, string].Ok(...)`). The copies are made in the program that uses them, once (R6). A library's generic is checked in the library's terms when it is compiled into a program; a library's copy that breaks a contract written in the program is reported at the contract with the "made by" line in the program.

## Rulings

Each with what it costs if wrong.

- **G1. Brackets after a name are settled by a syntax pass right after parsing and linking, not in the checker.** It applies the proposal's rule (by what the name is; a local never hides an item) with the names already resolved by `link`, so the checker and KIR see only `Index`, `TypeCall` and the new `TypeApply`. If wrong: none visible; settling in the checker would teach the ~40 places that match `Index` a new node.
- **G2. A generic type's bare name inside its own declaration means it with its own placeholders.** If wrong: methods write `Stack[T]` (the parser's `self` type would need the placeholders filled in); one branch in `resolve_type`.
- **G3. Placeholder names start with a capital letter.** If wrong: one check to drop.
- **G4. `ordered` and `key` include `equatable`.** If wrong: `[T: ordered + equatable]` to compare.
- **G5. `<`, `<=`, `>`, `>=` work on two strings and two bools everywhere.** If wrong: `ordered` gives only sorting; a few lines in `ops.rs` and the KIR comparison.
- **G6. A type holding a function anywhere can't be `T`** (lists of functions included), so "may hold a function" is "no" for `T` without exceptions. If wrong: a generic queue of callbacks waits a round (as R3 already says).
- **G7. Inference order: values, the expected type (filling only), deferred arguments (closures, `none`, `[]`, `{}`), closure results; `var` arguments, parts inside types and closure results bind exactly.** If wrong: some calls need written types; the order is one function.
- **G8. Written type arguments give all placeholders.** If wrong: partial ones (`count_by[_, string]`) later.
- **G9. Rules whose methods share a name and signature share the method; differing signatures are an error at the declaration.** If wrong: an error that could have been a call-site ambiguity.
- **G10. Endless copies: a static check at declarations (Go's "instantiation cycle", `types2/monomorph.go`: an edge from each placeholder to each placeholder its type arguments feed, marked growing when the argument isn't the placeholder itself; a cycle with a growing edge is an error), over field types for structs and enums and over written or worked-out type arguments of calls in generic code; plus a cap during collection (32 levels of nesting in a type argument, or 10,000 copies of one generic) for cycles through a rule's method, which the static graph can't see.** If wrong: the static check can refuse a rare valid pattern where a call grows a type and a rule call shrinks it back (`h[V]` calls `g[Box[V]]` whose rule call reaches `Box[V].m` calling `h[V]`); the cap can stop a valid program that nests more than 32 levels, or whose type lists grow past 1,000 parts (P41). Known gap: a method's edges use its type's placeholders, which all its methods share, so `Stack[T].flat` calling `Stack[list[T]]().size()` is refused ("`Stack` would need endless copies: `Stack[T]` calls `Stack[list[T]]`, ...") although `size` never calls `flat`; Go's check is as coarse.
- **G11. Copies are new `FnId`s appended to `Tables::fns` (and instance structs and enums new `StructId`s and `EnumId`s), so KIR's `FuncIdx == FnId`, `StructIdx == StructId` and `EnumIdx == EnumId` stay true; a generic original is lowered as a stub that traps.** If wrong: a few unused stub functions in KIR.
- **G12. Generic types are instances interned per (generic, type arguments) with their fields filled in, so every type question works unchanged; an instance's owning-ness is its declaration's.** If wrong: memory per instance (small).
- **G13. Lowering never makes a checker type: the collection pass interns every substituted type a copy's lowering reads, and lowering's substitution only looks types up, panicking on a miss** ("the collection pass missed `list[T]` in `best[Enemy]`; this is a compiler bug"). If wrong: a missed case panics loudly in lowering instead of miscompiling.
- **G14. In a copy, a recorded conversion or borrow whose source and target are the same type is dropped** (a `T: Damageable` value passed as `Damageable` when `T` is `Damageable`). If wrong: none known.
- **G15. The report lists each generic function or method with no copies as "not checked, no copies"; an item inlined from a copy is attributed to the generic by its plain name.** If wrong: report wording only.
- **G16. A placeholder can't share a name with any type; functions and constants may; `link` leaves placeholder names alone in types.** If wrong: one check.
- **G17. `print` shows an instance as `Stack[int]`, each argument as `print` names types.** If wrong: output text only.
- **G18. A placeholder no parameter mentions is allowed; it must be written.** If wrong: a warning could say so.
- **G19. `best[int]` is a constant expression** (a function value). If wrong: one arm of `is_const_expr`.
- **G20. A rule or R3 error goes at the argument that decided the placeholder, else at the written type, else the call.** If wrong: error positions only.
- **G21. "Made by" is the first place, in walk order of non-generic code, that led to the copy.** If wrong: another line in the message.
- **G22. No placeholders on methods, interfaces (R8), natives or `main`;** an interface can't be named `equatable`, `ordered` or `key`. If wrong: none this round.
- **G23. A deferred closure is checked after the other arguments.** Order can't change meaning: a closure can't move or change what it captures, and owning values can't be captured. If wrong: none known.
- **G24. Generic bodies, field defaults of generic types and their closures are checked once; copies share the generic's locals (`LocalId`s) with substituted types.** If wrong: none visible.
- **G25. A copy's purity, `io` and `mutates_params` are the generic's.** Calls through a rule are judged by the interface's method, which is at least as strict as any copy. If wrong: none.
- **G26. The "can't tell" message writes the decided placeholders and `int` for the rest.** If wrong: wording.
- **G27. `kestrel check` reports nothing for a generic with no copies** (no warning). If wrong: a warning could say a library function is unused.

## How it is built

Guidance for the plan; the builder may change details that don't change behaviour. Several files are near the 400-line limit (`check/src/tables.rs` 383, `body/lists.rs` 348, `kir/src/lower/mod.rs` 346); new code goes into new files named below.

### Syntax (`kestrel/syntax`)

- **AST** (`ast.rs`): `pub struct TypeParam { name: String, span: Span, rules: Vec<(String, Span)> }`; `type_params: Vec<TypeParam>` on `StructDecl`, `EnumDecl`, `FnDecl` (empty for everything existing). Two expression kinds:
  - `ExprKind::Brackets { base: Box<Expr>, index: Option<Box<Expr>>, types: Vec<TypeExpr> }`: the undecided node, made by the parser only; never seen by the checker.
  - `ExprKind::TypeApply { name: String, name_span: Span, type_args: Vec<TypeExpr> }`: a generic named with types and not called (`Result[int, string]` before `.Ok`, `best[Enemy]` as a value).
  - A called one reuses `ExprKind::TypeCall { name, name_span, type_args, args }`, which `array[int](n)` already uses; the checker and KIR tell built-in from user names, and every arm handles a user `TypeCall` exactly like the matching `Call` (today's arms treat every `TypeCall` as `array`/`list`/`map`).
- **Parser** (`parser/generics.rs`, new): `type_params()` after a struct, enum or function name (`item.rs`, `members.rs`): `[` Name (`:` Rule (`+` Rule)*)? (`,` ...)* `]`, rules as identifiers. In a struct or enum body, `name[` is "methods can't have placeholders of their own yet; use a generic function that takes the struct instead"; after `interface Name`, `[` is "interfaces don't take placeholders yet"; `native name[` is "native functions can't have placeholders", and the native item is dropped (the driver's check that only built-in modules declare natives would otherwise add a second error).
- **Brackets after a name** (`parser/expr.rs`, `postfix`): when `[` follows an `Ident` or a `Field`, parse the contents twice. First as a type list (`type ( , type )* ]`) silently: save the position, diagnostics length and depth counters, parse, record the result, restore (types hold no expressions, so nothing else changes). If the type list fails, parse today's index expression for real at once: today's `Index`, or today's errors (no second attempt, so malformed nested brackets cost linear work per level, not exponential). If the type list parses, try the index inside an attempt. If both parse: `Brackets { index: Some, types }`. If only the type list does (`Result[int, string]`, `best[list[int]]`): restore to the bracket, parse the types again for real, `Brackets { index: None, types }`. A failed index attempt is undone with a `Parser::attempt` helper that restores the position, diagnostics, depth counters, `hex_literals` and `proves` lengths and `block_heights` (a failed closure body leaves a block open); ids it handed out stay unused (side tables size by the count). The postfix loop then continues as today, so `best[Enemy](foes)` is `CallValue { callee: Brackets }` and `Result[int, string].Ok(..)` is `MethodCall { receiver: Brackets }`.
- **`link`** (`link/walk.rs`): placeholder names in scope for an item are left alone in every type in the item, its bodies included (`let` annotations, closure parameters, `TypeCall` and `Brackets` types), and suppress `unseen`; rule names are rewritten like interface names except the three built-in words. For `Brackets`: rewrite the base first (`geom.best` becomes `Ident("geom.best")` as today); if the base is now an `Ident` naming a struct, enum, interface or function item and isn't a local in scope, rewrite `types` (with `unseen` reports); otherwise rewrite `index`. `Export` gains the item's kind for this.
- **Settling** (`settle.rs`, new): one pass, run at the end of `link()` and of `parse()` (single-file callers such as the checker's tests); `parse_at` (used before linking) doesn't. It walks the program with a stack of locals in scope (parameters, `let`s, loop variables, `match` bindings, closure parameters) and the set of item names by kind, and rewrites each `Brackets` whose base is an `Ident` naming a struct, enum, interface or function item, not a local: inside `CallValue { callee: Brackets }` the whole call becomes `TypeCall { name, type_args: types, args }` (the call's id); otherwise the node becomes `TypeApply` (its own id). Every other `Brackets` becomes `Index { base, index }`, or, when `index` is `None`, the error "`[...]` after `xs` indexes it, but `int, string` isn't a value; types go after a generic type or function". The link walk and this pass share one decision function.
- `sexpr.rs` prints the new nodes and type parameters; `depth.rs` counts them.
- **A visitor** (`visit.rs`, new): `each_expr(block, f)` calling `f` on every expression of a block, closure bodies, interpolation parts and `@assume` conditions included. The collection pass and the expansion check use it.

### Checker: representation (`check/src/generic/`, new)

- **`Ty::Param(ParamId)`** (`ty.rs`), `Ty` stays `Copy`. `Tables::params: Vec<ParamInfo>`, `ParamInfo { name, span, owner: GenericOwner, index: u32, ifaces: Vec<IfaceId>, equatable: bool, ordered: bool, key: bool }` (the implied `equatable` already set). `GenericOwner = Struct(StructId) | Enum(EnumId) | Fn(FnId)`.
- **Type lists:** `TyListId`, interned in `Tables::ty_lists` / `ty_list_ids` like `ElemId`.
- **Instances (G12):** a generic declaration is a `StructId` / `EnumId` whose `StructInfo::params: Vec<ParamId>` isn't empty; its fields use `Ty::Param`, and as a type it means itself with its own placeholders (`Stack[T]`). Every other use is an **instance**: `StructInfo::instance: Some(Instance { def, args: TyListId })`, interned in `Tables::struct_instances: HashMap<(StructId, TyListId), StructId>` (and `enum_instances`); `intern_struct(def, args)` returns `def` itself when `args` are its own placeholders. An instance copies the declaration's `item`, `span`, `methods` (the generic `FnId`s) and `ifaces`, gets `name` = `Stack[int]` (from `ty_name` of the arguments), `fields` substituted (each keeping the declaration's `has_default` and `default_allocates`), and `concrete: bool` (no `Param` anywhere). Instances interned during `collect` are synced as the declaration's facts arrive: `methods` when pending instances are filled (methods are added per item in source order), `ifaces` after conformance (which fills them), and the per-field flags after default allocation (which sets `default_allocates`). A generic declaration has `concrete: false`. `Ty::Struct(id)` of an instance answers every question as any struct does, because its fields are real (`has_heap`, `holds_*`, `zero_fill_breaks`, `contained`, `ty_name`, field lookup, `implements`). Instances with placeholders (`Stack[U]` inside `best[U]`) exist only in checker types; KIR never sees them.
- **Owning:** `compute_owning` runs over declarations with `Ty::Param` not owning; an instance's entry in `owning_structs` / `owning_enums` is set from its declaration when it is interned, at any time (the R2 concern: an instance first seen in a body is owning exactly when its declaration is, since R3 keeps owning types out of placeholders).
- **Substitution:** `Subst { owner: GenericOwner, args: TyListId }` maps the owner's placeholders by index. `Tables::subst(&mut self, ty, s) -> Ty` (checker and collection: interns elements, signatures, type lists and instances; maps `Ty::Struct`, `Ty::Enum` and `Ty::Variant` of a declaration or a non-concrete instance; a fast path returns `ty` when `is_concrete(ty)`, since `Ty::Struct(def)` and `Stack[U]` hold no `Param` themselves), and `Tables::subst_known(&self, ty, s) -> Ty` (KIR: the same walk, lookups only, panicking on a miss, G13). `Tables::is_concrete(ty)`.
- **Type questions** (`tables_ty.rs`): `Ty::Param` gets the R4 answers in the table above: `has_heap` and `zero_fill_breaks` true, `holds_where` true for `holds_iface` and for `holds_uncomparable` unless `equatable`, false for `holds_fn`, `is_owning` and `holds_arena` false, `Ty::is_aggregate` true, `vectors::vector` none, `coerces_to` only itself, `contained` nothing, `ty_name` the placeholder's name. `map_ty` takes a `Param` with `key`; `sortable` in `list_fns.rs` and the `has`/`find` comparability in `lists.rs` take a `Param` with `ordered` / `equatable`.
- **`FnInfo`** gains `generic: Option<GenericOwner>` (the owner whose placeholders its types use: its own for a generic function, its struct's or enum's for a method), `type_params: Vec<ParamId>` (a generic function's own) and `copy: Option<CopyInfo { generic: FnId, args: TyListId, made_at: Span, made_by: Option<FnId> }>`. `Tables::fn_copies: HashMap<(FnId, TyListId), FnId>`.
- **Side tables:** `Tables::type_args: HashMap<ExprId, TyListId>`: for every call, method call, type call and function value whose callee is generic, the callee owner's placeholders' types in the caller's terms (for a method, the receiver instance's arguments). `Res::ParamCall { param: ParamId, iface: IfaceId, slot: u32 }`: a method called on a `T` through an interface rule.

### Checker: declarations (`collect.rs`, `collect_ty.rs`, `generic/declare.rs`)

`collect` runs in phases:
1. Register every struct, enum and interface by name (as today), and each generic struct's and enum's placeholders as `ParamInfo`s; check placeholder names (capital, not a type, not repeated). A function's placeholders are registered inside `add_fn` in step 3, where its `FnId` (the owner) is made, with the same checks and its rules.
2. Resolve rules (interface names or the three words; repeats) and refuse interfaces named after a rule.
3. Resolve fields and signatures (and add every function, method and interface method, as today's members loop does). `resolve_type` takes a type scope (the placeholders in scope by name: the struct's or enum's for its fields and methods, the function's own for a generic function) and handles `Named { name, args }` for generic declarations: the count must match ("`Pair` takes 2 types, found 1"), a non-generic name with arguments is "`Enemy` takes no types", a bare generic name outside its own declaration is "`Stack` needs its types, like `Stack[int]`". An instance named here is interned **pending** (fields not yet filled), and its arguments are queued for the R3/R7 check (owning-ness and fields aren't known yet). Then the **G9 clash check**, which needs the interface methods' signatures made in this step: methods sharing a name must have the same `self`, parameters and result, compared symmetrically with `pure` ignored; a call through `T` is `pure` when any rule's method is, so `[T: A + B]` and `[T: B + A]` behave the same.
4. **The type expansion check** over generic declarations' field types (G10). A declaration in a growing cycle is reported and its pending instances left empty (fields `Error`), so nothing below loops.
5. Fill pending instances (a worklist; finishes because of step 4), copying the declaration's `methods` again.
6. Conformance (as today; a generic type's methods must fit for every `T`, so `hp(self) -> T` doesn't fit `hp(self) -> int`), then every existing instance's `ifaces` copied from its declaration; default allocation, then every existing instance's per-field `has_default` and `default_allocates` copied from its declaration (otherwise an instance named in a signature would build its defaults in no arena: a use after free); self-containment over declarations and plain structs (an instance contains itself only if its declaration or a plain struct around it does), natives.
7. `compute_owning`, then the queued R3/R7 checks.

In bodies, `resolve_type` interns and fills an instance at once and checks R3/R7 immediately. `check_default_cycles` treats a construction of an instance as one of its declaration.

### Checker: bodies (R1)

- `check_fn` runs on generic functions and methods once, with the type scope of their owner in `BodyChecker` (`type_scope`). Closures, `let` annotations and type calls resolve types with it.
- **Methods on `T`** (`body/param_calls.rs`, new): in `check_method_call`, a receiver of type `Param(p)` looks the name up in `p`'s interface rules (G9 decides clashes) and records `Res::ParamCall` (with the first rule whose method is `pure`, else the first that has it); the call is then checked as an interface call today is (`check_args` with the interface's abstract method, `self` skipped; `var self` needs a changeable receiver; purity by the interface's method).
- **Operators** (`ops.rs`): `==`/`!=` on two values of one `Param` type need `equatable`; `<`.. need `ordered`; any other operator on a `Param` gets the operator message. Two strings or two bools with `<`.. are allowed (G5).
- **Conversions:** `coerce` accepts `Param(p)` to `Iface(i)` when `p` has rule `i` (recorded in `coerce`, and in `iface_borrows` by the existing place rule), and to `Optional(Param(p))`.
- **Fields of `T`, `T` as a value or a constructor, `array[T](n)`:** the messages above, in `check_field`, `ident`, `check_call` and `check_type_call`.
- `match` needs nothing new: an instance's variants have filled-in fields.
- **Generic calls with types written** (`body/generic_call.rs`, new): `check_type_call` on a user name dispatches to a generic construction or call; `TypeApply` as a method-call receiver or a field base (`enum_named`) gives an enum instance; as a value, a function value with `type_args` or the "is a type" error. Methods on instances: the method's signature substituted with the receiver instance's arguments (`type_args` recorded).
- **`check_args` gains a pre-checked mode,** like `check_construct_with`: inference checks the arguments once, then the rest (`var` arguments, stack closures, exclusivity, later-change checks, `call_args`) runs against the filled-in signature without checking them again. `check_method_call`, `check_type_call` and `variant_construct` take the expected type, as `check_call` already does.

### Checker: inference (`generic/infer.rs`, new)

`infer(callee placeholders, parameter types, arguments, expected) -> Result<TyListId, Reported>` implements the four steps above. Bindings are `Vec<Option<(Ty, exact: bool, from: arg index)>>`. Matching is one-sided: placeholders on the callee's side are unknowns; types on the caller's side are opaque, even when the caller is the same generic (`walk[T]` calling `walk` binds the callee's `T` to the caller's `T`). Substitution is applied once, never repeated. A deferred closure gets its expected function type with known parameter types; when its result is open, the closure is checked with the existing `infer_result` path (as for `map`) and its result binds exactly. The undecided and conflict messages come from here.

### The expansion check for functions and the collection pass (R2, `generic/copies.rs`, new)

Runs in `check_keeping_tables` (which `check_with` wraps, and which the editor's `analyze` calls) after every body is checked and only when there are no errors so far (like `check_default_cycles`); it returns at once when the program declares no generic:

1. **Static check (G10)** over each generic function's and method's `type_args` entries for calls, function values and method calls (with the receivers' arguments), not constructions or variants: edges from caller placeholders to callee placeholders, growing when the argument isn't the placeholder itself; a growing cycle is the "`walk` would need endless copies" error at the declaration. (A construction like `Box(v: self)` in `Box.grow` would make a growing self-edge and refuse a program whose cycle the cap is meant to report; growth through constructions is the cap's.)
2. **Roots:** every non-generic, non-abstract function body, every constant initializer, every non-generic struct's field defaults (closures there may call generics). `@assume` conditions are part of bodies. Kestrel has no global variables.
3. **Walk** each body with `each_expr` under its substitution (none for roots) and, for each expression: intern `subst(expr_ty)`, `subst(coerce)`, the types of `type_args`; for `Res::Fn` / `Res::FnValue` of a generic callee, request the copy `(callee, subst(type_args))`; for `Res::ParamCall`, find `T`'s concrete type: a struct or enum gets its method of that name (a copy when it is an instance), an interface type needs nothing; for `Res::Construct(sid)` of an instance with missing fields, walk the declaration's defaults under the instance's substitution (once per instance); for a conversion from a concrete instance to an interface (or its optional), request copies of every method the interface's table needs. Also intern the substituted types of the body's locals (`FnInfo::locals` and each closure's `locals`), closure signatures and capture types.
4. **A requested copy** that doesn't exist gets a new `FnInfo` appended to `t.fns`: name (`best[Enemy]`, `Stack[int].push`), `item`/`method`/`span`/`locals` of the generic, `params`/`ret`/`sig` substituted, `is_pure`/`io`/`mutates_params` the generic's (G25), `copy` with `made_at` (the root expression's span) and `made_by` (the requesting copy, for the chain in messages); entered in `fn_copies` and the worklist. The walk repeats until the worklist is empty.
5. **The cap (G10):** a type argument nested more than 32 levels, a type list that would grow past 1,000 parts (P41: measured before interning, since names double with the type), or more than 10,000 copies of one generic, stops with "making copies of `walk` doesn't end ..." showing the chain through `made_by`. The 32-level cap applies to every type the walk interns, not only copy requests: walking field defaults under each instance's substitution can grow a type with no function request (`struct S[T] { f: pure () -> int = () { let inner = S[list[T]](); 1 } }`), and stops with "making copies of `S` doesn't end: `S[int]` needs `S[list[int]]`, and so on (stopped after 32 levels)".

After it, every type a copy's lowering reads is interned (G13), and the tables are frozen.

### KIR lowering (`kir/src/lower`)

- **One substitution-aware accessor** (`lower/subst.rs`, new). `FnLowerer` gets `subst: Option<Subst>` (the copy's) and a stack for field defaults. Every read of a checker type or resolution in lowering goes through it:
  - `ty(e: ExprId) -> Ty` (replaces `t.ty(e)`, 26 uses), `sub(ty) -> Ty` (any `Ty` read from the tables: a local's, a closure's signature, a capture's, a parameter of a generic `FnInfo`), `local_ty(l)`, `coerce(e) -> Option<Ty>` (dropped when the substituted source equals the target, G14), `borrowed(e) -> bool` (`iface_borrows`, dropped likewise);
  - `res(e) -> Option<Res>` (replaces the ~16 reads of `t.res`): `Fn`/`FnValue` of a generic callee become the copy's `FnId` via `fn_copies[(f, subst(type_args[e]))]`; `Construct(sid)`, `Variant(eid, v)` and `CallValue(sig)` get substituted ids; `ParamCall` becomes `Res::Fn(the concrete type's method or its copy)` or `Res::IfaceCall { iface, slot }` when `T` is an interface type, so the existing dispatch in `expr.rs`, `stmt.rs` and `call.rs` handles calls on `T` as direct (inlinable) calls;
  - `with_subst(s, f)`: lowering a missing field's default in `construct` (`call.rs`) runs under the instance's substitution.
  - A source test (`kir/tests/subst_discipline.rs`) fails if `t.ty(`, `t.res.get(`, `t.res[`, `t.coerce.get(` or `iface_borrows.contains(` appears in `kir/src/lower` outside `subst.rs`.
- **Ids that map straight across** stay right because copies and instances have their own checker ids (G11): `FuncIdx(f.0)` in `call_parts`, `fn_ref`/`thunk_for`, `MakeIface` tables and `prove.rs` receive copy ids from `res()`; `StructIdx(sid.0)`/`EnumIdx(id.0)` in `construct`, `variant_construct` and unit variants receive substituted ids; `kty` receives substituted types and panics on `Ty::Param` with "a generic type reached KIR unsubstituted; this is a compiler bug".
- **Per-copy functions:** `lower()` lowers every `FnInfo`; a generic original (`generic` set, `copy` none) becomes a stub (no parameters, `Trap`, `lines: (0, 0)`, not exported); a copy is lowered from the generic's declaration with `subst = Some(copy's)`, `exported: false`. `layout::layouts` gives non-concrete structs and enums an empty placeholder layout, and `Module::elems` a placeholder for non-concrete elements, so the indexes line up; nothing refers to them.
- **Interface tables** (`lower/tables.rs`): `table_for` on an instance maps each interface method's name to the instance's method copy through `fn_copies`. `iface::implementations` returns copies of concrete instances' methods (those that exist) and skips generic originals and non-concrete instances.
- **Per-copy helpers:** everything keyed by a closure literal's `ExprId` is keyed by `(ExprId, Option<FnId>)` with the copy being lowered: `Helpers::closures`, `env_copies`, `env_layouts`; `Callee::Value(SigId)` gets the substituted signature; `@prove` loop sites are made per (annotation, copy) (`Proves::loops`). `par for` bodies, thunks and sort comparisons are made per lowering already.
- **Comparisons (G5):** `<`.. on two strings lower to `StrOp::Cmp` (the existing `kestrel_rt_str_cmp`) compared with 0; on two bools to a comparison of their `u8` values. Both work in copies and in plain code.
- Range analysis, the inliner, `scalarize`, `promote` and codegen are unchanged: they see ordinary functions.

### Names, report and contracts

- `Func` gains `copy: Option<CopyNote { generic: String, made_by: String, made_at: u32 }>` (`generic` is the plain name, `best` or `Stack.push`; `made_by` is `Stack[string]` for a method copy, `best[Enemy]` for a function copy; `made_at` a packed line). `Module` gains `uncopied: Vec<(String, (u32, u32))>`: generic functions and methods with no copies, by name and lines.
- `contracts/words.rs::target` appends " (made by `X` on line N)" (or "on geom.kes:N") for a copy. `Proves::new` skips generic originals, so each copy of an annotated generic gets its own site (copies share the declaration's span).
- `report.rs`: copies are ordinary `User` functions named by `display_name` (which already strips a module prefix); `Owners` leaves copies out and keeps one span per generic declaration (its copies' lines) under `CopyNote::generic` (G15), so an item inlined from a copy names the generic; closures and `par for` bodies in copies are attached by `made_closures` (the function that makes them). Each entry of `uncopied` is listed as "`best`: not checked, no copies".
- `layout.rs::shown` builds an instance's print name from its declaration's shown name and its arguments' shown names (`geom.Stack[geom.Item]` prints as `Stack[Item]`).

### Modules

Nothing beyond `link` and `settle` above: the linked program is one program, so copies are per program (R6) and names are qualified (`geom.Stack`). `cli/tests/modules.rs` covers it.

## Tests

Program tests (`kestrel/tests/programs/generic_*.kes`, JIT and exe, poisoned as every program is), each with `// expect:` lines:
- `generic_stack.kes`: `Stack[int]`, `Stack[string]`, `Stack[Enemy]`, `push`/`pop`, a `list[Stack[int]]`, a `Stack[Stack[int]]`, printing a stack (`Stack[int](items: [3])`), a copy of a stack changed separately (string items deep-copied).
- `generic_best.kes`: `best` over `list[Enemy]` and `list[int]` with closures, a named function, a stored closure, `best[Enemy](...)` written, `let f = best[Enemy]`, `let RULE = best[int]` as a constant.
- `generic_rules.kes`: `total_hp` over goblins (direct calls), over `list[Damageable]` with `T = Damageable` (indirect), `[T: Damageable + Named]`, a `var self` rule method changing the caller's value, a `T` passed where `Damageable` is expected.
- `generic_builtin_rules.kes`: `count_by` with `string` and `int` keys, `[T: equatable]` `has`/`find`/`==` over ints, strings and `vec2`s, `[T: ordered]` `sort()` and `<` over floats, strings and bools, `sort_by` with a `T` key.
- `string_bool_compare.kes`: `"apple" < "banana"`, non-ASCII order, `false < true`, in plain code (G5).
- `generic_result.kes`: `enum Result[T, E]`, `match` in plain code and inside a generic function (`unwrap_or[T, E]`), unit variants with the expected type (`Maybe[int].Nothing` and `let m: Maybe[int] = Maybe.Nothing`), `Result[int, string].Ok(value: 3)` written, a generic enum's method.
- `generic_infer.kes`: each inference step: values deciding, `int` then `float` widening, the expected type filling (`let s: Stack[int] = Stack()`), closure results (`make(() { 3 })`), a closure parameter typed by an earlier argument, `none` and `[]` deferred.
- `generic_defaults.kes`: a generic struct with `items: list[T] = []` and a closure default calling a generic function, built in a function and in a constant; labelled construction in another order.
- `generic_closures.kes`: closures in generic code capturing `T` values (stack and heap environments), `filter`/`map`/`any`/`all`/`sort_by` on `list[T]`, a closure stored in a `Stack[T]` field of type `(T) -> int` built per copy, `par for` over a `list[T]` calling a pure rule method.
- `generic_arena.kes`: a `Stack[string]` filled inside `in w.mem { }` and returned out of `arena { }` (refused in an error test, used inside it here), with `KESTREL_POISON` as usual.
- `generic_iface_impl.kes`: `struct Stack[T]: Named` converted to `Named` in plain code and inside a generic, one table per instance.
- `generic_recursion.kes`: a generic function calling itself with the same types, mutual recursion of two generics, a generic method calling another method of its struct.

Error tests (`kestrel/tests/errors/generic_*.kes`), each message with `// error <line>: <message>`: one file each for declarations (placeholder names, rules, G9, methods/interfaces/natives/`main` with placeholders, an interface named `key`), types (counts, bare names, "takes no types", R3 for owning/optional/function/holds-function, R7 including a caller placeholder missing the rule), bodies (field of `T`, method not in rules, no rules, operators, `==`/`<`/map key/`sort`/`has` without rules, `array[T](n)`, `T()` and `T` as a value, a generic function as a value, `Stack[int]` as a value, a non-pure rule method in a `pure` function and in `par for`), inference (each undecided form, conflicts, the exact case, a closure whose parameter stays unknown), endless copies (type, function, and one through a rule method hitting the cap), self-containment through a generic (`Bad`, `L[T]`), and brackets (`xs[int, string]` on a local list).

Unit tests:
- **syntax:** placeholder lists and rules in `sexpr` form; the dual parse (`xs[i]`, `best[Enemy](foes)`, `Result[int, string].Ok(..)`, `geom.Stack[int]()`, `handlers[i](3)`, `best[list[int]]`, `m["k"]`, `xs[-1]`, nested `xs[ys[i]]`); `settle` with a local named like an item in another module; the failed-attempt rollback (diagnostics, ids unused, `hex_literals`); the R8 parser messages.
- **check** (`check/tests/generics_*.rs`): instance interning (one id per type, `def` returned for its own placeholders), substitution, the R4 answers per question, owning instances first seen in a body, the type expansion check, inference results and bindings, `type_args` recorded for calls and methods, the collection pass (copies made, deduplicated, `made_at`, copies of interface methods for a converted instance, nothing for an unused generic, the cap's chain).
- **kir** (`kir/tests/generics.rs`): one function per copy with substituted types, calls on `T` lowered as direct `Call`s (and `CallVirtual` for an interface `T`), one closure code per (literal, copy), interface tables per instance, stubs for originals, the subst-discipline source test, `<` on strings as `StrOp::Cmp`.
- **cli:** `modules.rs` (a `pub` generic struct, enum and function imported both ways, `geom.best[Enemy](..)`, one copy for two modules, module names in messages and print); `report.rs` (per-copy entries, "not checked, no copies", an inlined item from a copy named by the generic); `contracts.rs` (`@prove(no_alloc)` holding for `Stack[int].push` and failing for `Stack[string].push` with the "made by" clause, a contract in the program broken by a library's copy, a generic with no copies passing).

**A measurement** in `kestrel/bench/kernels`: the `iface_step` kernel written as `total_hp[T: Damageable]` over a `list[Goblin]`, against the `list[Damageable]` loop and the hand-written `list[Goblin]` loop, so "runs like the copy you'd write by hand" is timed, not assumed. Results in `results.md`, losses included.

## Tasks (build order)

Each task leaves `cargo test --workspace` and clippy green. Until Task 6, the driver (`compile_reading`, which `compile`, `compile_as`, `compile_as_reading` and `compile_unoptimized` all go through) refuses a program that declares a generic with "generics aren't supported yet", so nothing reaches KIR half-built; the checker's own unit tests see the real checker from Task 2 on.

1. **Parsing, linking, settling.** AST (`TypeParam`, `Brackets`, `TypeApply`), `parser/generics.rs`, the dual parse with `Parser::attempt`, `link` changes, `settle.rs`, `visit.rs`, `sexpr`/`depth`. Delivers: every new form parses and settles; the checker refuses generics with a temporary error until Task 2 (then only the driver's refusal remains). Tested by syntax unit tests and the existing suite (indexing unchanged).
2. **Checker representation and declarations.** `Ty::Param`, `ParamInfo`, type lists, instances, `subst`/`subst_known`, the R4 answers, `resolve_type` with a type scope, the collect phases, the type expansion check, R3/R7 on written types, instance names. Delivers: generic declarations and written instance types check; bodies of generic code still refused. Tested by check unit tests and the declaration and type error tests.
3. **Generic bodies and written types.** `type_scope` in `BodyChecker`, `Res::ParamCall`, the operator and rule messages, conversions to rule interfaces, `match` on generic enums, field defaults of generic structs, closures and `par for` with `T`, `TypeCall`/`TypeApply` with written types, methods on instances, `check_args`'s pre-checked mode. Delivers: R1 (generic code checked once) with every call's types written. Tested by check unit tests and the body error tests.
4. **Inference.** `generic/infer.rs` and its use in calls, constructions and variants; expected types threaded into method calls and type calls. Delivers: pick 2 with every message. Tested by check unit tests and the inference error tests.
5. **The collection pass.** The static function expansion check, roots, the walk, copies as `FnInfo`s, `fn_copies`, `made_at`/`made_by`, the cap. Delivers: frozen tables with every copy (R2, R6). Tested by check unit tests and the endless-copy error tests (error tests go through the checker, which the driver runs before its refusal). Program tests start in Task 6, which lifts the driver's refusal.
6. **Lowering per copy.** `lower/subst.rs` and the migration of every read, stubs, placeholder layouts and elements, per-copy lowering, calls to copies, `ParamCall` to direct or virtual calls, defaults under the instance's substitution, interface tables per instance, `implementations`, the source test. Delivers: generic structs, enums and functions run. Tested by `generic_stack`, `generic_best`, `generic_rules`, `generic_result`, `generic_infer`, `generic_defaults`, `generic_iface_impl`, `generic_recursion` and KIR tests.
7. **Per-copy helpers.** Closure code, environments and layouts per (literal, copy), list-method loops and sort keys with substituted signatures, `par for` in copies, function values and thunks of copies, `@prove` loop sites per copy. Tested by `generic_closures`, `generic_arena` and KIR tests.
8. **Ordered and equatable comparisons.** `<`.. on strings and bools in the checker and KIR (G5), `equatable`/`ordered`/`key` operations in copies (`==` on vectors and strings, `has`/`find`, `sort()`, map keys). Tested by `string_bool_compare`, `generic_builtin_rules` and error tests. (Independent of 6-7 on the checker side; its lowering part needs 6.)
9. **Names, contracts and the report.** `Func::copy`, `Module::uncopied`, the "made by" clause, `Proves::new` over copies, the report's per-copy entries, `Owners`, `shown` for instances. Tested by `cli/tests/report.rs` and `contracts.rs`.
10. **Modules.** Imported generics both ways, qualified bracket forms, one copy across modules, names. Tested by `cli/tests/modules.rs`.
11. **Docs and the measurement.** The syntax spec's "Generics" section (and its "Out of scope" line), the README, this spec's "As built"; the kernel above with `results.md`.
12. **The final whole-round review (Opus) and its fix wave,** then the full suite and clippy in all three workspaces when the machine is free.

Opus for Tasks 2, 4, 5 and 6 and every review (representation, inference, the collection pass and lowering are where generic code could become unsafe); Sonnet for 1, 7-11.

## Not this round

From the proposal: the `number` rule (question A); owning types, optionals, function types and types holding functions as `T` (R3, G6); placeholders on methods and interfaces (R8); default types (`[T = int]`); constant placeholders (`Grid[W, H]`); `is` type tests; specialising a generic for one type; Godot exports of generics. Also: partial written type arguments (G8); writing a module's type as `geom.Stack[int]` in a type position (types from modules are imported by name, as today); working out a generic function value's types from an expected function type (`let f: (list[Enemy]) -> Enemy? = best`); a standard `Result` module (the tests write their own); devirtualising calls on an interface `T`; sharing one copy between types with the same layout.

## Corrections from planning (2026-10-24)

The plan (`docs/superpowers/plans/2026-10-24-generics.md`, rulings P1-P37) found these while reading the code; it wins over the text above on each:
- **`<` on strings** compares `StrOp::Cmp` (which gives 0 for before, 1 for equal, 2 for after) with 1, not with 0 (P5).
- **A copy's contract message** is today's sentence ("`X` can't be proven `no_alloc`: it allocates on line N: ...") with " (made by `Stack[string]` on line 12)" after the target's name; the example above isn't today's wording (P6).
- **Per-copy caches in lowering** (closure code, environments, layouts, `@prove` loop sites) are keyed by the active substitution, not by `(ExprId, Option<FnId>)`: a closure in a generic struct's field default is lowered in plain code under the instance's types, so `Box[int]` and `Box[string]` would otherwise share one closure code (P4).
- **A non-empty array literal given for an open `list[...]` parameter** decides through its items and becomes a list (otherwise `count([1, 2])` can't work out `T`, and G7's own example can't happen) (P11).
- **Unit variants take the expected type** too (`let m: Maybe[int] = Maybe.Nothing` is a field access, so `check_field` gets it) (P8).
- **`is_const_expr`:** a `TypeCall` is constant only for the built-in names and for constructions of generic structs (`let EMPTY = Stack[int]()`); a generic function call like `best[int](xs)` is not a constant expression (P17).
- **Two more guards behind G13:** the subst-discipline test also forbids `t.sig(`, `t.local(` and `t.closures[` outside `subst.rs` (P19), and lowering refuses any non-concrete type at its end, not only a bare `Ty::Param` in `kty` (P16).
- **The new checker tables are one field, `Tables::generic`,** to keep `check/src/tables.rs` under 400 lines (P1).

From the plan review (2026-10-25, rulings P21-P37); the text above is updated to match:
- **The driver's refusal is in `compile_reading`,** not `compile_unoptimized`, which the CLI's and the editor's paths don't call (P21).
- **Instances interned during `collect` are synced:** `methods` when pending instances are filled, `ifaces` after conformance, and `has_default`/`default_allocates` after default allocation; otherwise an instance named in a signature builds its defaults in no arena (a use after free) and has no interfaces or methods (P22, P23).
- **The function expansion graph has edges for calls, function values and method calls only,** not constructions or variants (P24), and **the 32-level cap covers every type the collection walk interns**, with the type wording "making copies of `S` doesn't end: ..." (P25).
- **G9's clash check runs after the members are added** (step 3), comparing signatures symmetrically and ignoring `pure`; a call through `T` is `pure` if any rule's method is (P26).
- **A generic declaration is not `concrete`,** and `subst`'s fast path is `is_concrete`; `subst` maps the ids of declarations and non-concrete instances (P27).
- **A user `TypeCall` is handled like the matching `Call` in every checker and KIR arm** (P28).
- **A function's placeholders are registered in `add_fn`,** where its `FnId` is made, not in step 1 (P29).
- **The collection pass (and the first task's temporary checker refusal) run in `check_keeping_tables`,** not `check_with`, so the editor runs the pass on every clean edit; it returns at once without generics (P30).
- **Copy names are sanitised into symbols** (`Pair[int, bool].swap` has spaces and brackets) (P31), and **`@assume` warnings are aggregated across copies by span**, so an assumption one copy needs isn't called useless (P32).
- **The dual parse tries the index only when the types parse,** and `attempt` restores `block_heights` (P34); a native with placeholders is dropped after its error (P35); placeholders are left alone in every type of their item, bodies included (P37).
- **A mixed comparison is "`<` needs two numbers, two strings or two bools, found string and int"** (P36).

From Task 3a's review (2026-10-25, plan rulings P38-P39, built in the plan's new Task 3c):
- **G28. Stores into `var` parameters are checked where the function is called** (P38), so generic containers work: `Stack[T].push(var self, v: T) { self.items.add(v) }` checks once (R1), as plain `push(var self, v: string)` now does too.
  - Each function records which of its `var` parameters it may store caller-arena memory into.
  - Each direct call checks that store against the place passed, the way built-in `add` is checked.
  - A call through an interface or a rule takes the union of the implementations' stores.
  - A storing function can't be a function value, and closures' own `var` parameters stay strict.

  R4's "arena escape rules as for a `string`" still holds: the summary assumes any `T` holds memory, so `Stack[int].push` inside `arena { }` on an outer stack is refused like `Stack[string]`'s. The whole replacement of a `var a: T` (`a = b`) stays refused when `T` may be an interface type (a copy lends the caller's concrete place), and is allowed when `T` has a built-in rule or two different interface rules, which no interface type can meet (R7, P39): `swap[T: equatable]` works. If wrong: a use after free through an unchecked indirect call; the plan's Task 3c lists each indirect path.
