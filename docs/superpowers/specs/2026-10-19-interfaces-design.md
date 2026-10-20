# Interfaces

Status: designed 2026-10-19. The user took the proposal's picks (`docs/superpowers/plans/2026-10-18-closures-interfaces-proposal.md`, section 2) on 2026-10-18 and said to build them after closures ("Ok go bro"). `is` type tests wait (the proposal's open question, answered "later"). Rulings that aren't the user's picks are Claude's, listed at the end with what each costs if wrong.

## Why

A `list[Goblin]` holds only goblins. A game with goblins, slimes and barrels that all take damage needs three lists and three loops, or one big `enum` whose every method is a `match`. GDScript does it with duck typing (`if body.has_method("take_damage")`), unchecked until the line runs. An interface is the checked version: a named list of methods; any struct or enum that declares it can stand in for it, and the compiler checks the promise.

## Syntax

```
interface Damageable {
    take_damage(var self, amount: int)
    pure hp(self) -> int
}

struct Goblin: Damageable {
    health: int = 10
    take_damage(var self, amount: int) { self.health -= amount }
    pure hp(self) -> int { self.health }
}

enum Barrel: Damageable, Drawable { ... }

var foes: list[Damageable] = []
foes.add(Goblin())
foes.add(Slime(size: 3))
for f in foes { f.take_damage(2) }
```

- **Declaring:** `[pub] interface Name { members }`. Each member is a method signature with no body, one per line: `[pure] name(self | var self, params) [-> Type]`. The first parameter is `self` or `var self`. At least one method ("an interface needs at least one method"). No fields, no default bodies, no duplicate names.
- **Implementing:** `struct Name: A, B { ... }` and `enum Name: A { ... }`, only where the type is declared. Interface names resolve like type names (`use { Shape from shapes }` brings one in).
- **Conformance:** for each method of each listed interface, the type has a method with the same name, the same `self`/`var self`, the same parameter types and the same result. A `pure` implementation satisfies a non-`pure` declaration; a non-`pure` one can't satisfy a `pure` declaration. Extra methods are fine. Messages:
  - "`Goblin` says it is a `Damageable` but has no method `hp`; add `pure hp(self) -> int`"
  - "`Goblin.hp` doesn't match `Damageable.hp`: it should be `pure hp(self) -> int`"
  - "`Damageable` is listed twice"; "`Thing` isn't an interface" (for `struct Goblin: Thing` where `Thing` is a struct or unknown).
- `interface` becomes a keyword.

## Using an interface as a type

An interface is a type like any other: parameters, results, `let`/`var`, fields, `list[Damageable]`, `[Damageable]`, map values, `Damageable?`.

- **Coercion:** a value of a struct or enum that implements `I` goes wherever an `I` is expected: an annotated `let`/`var`, an argument, a field or element store, `add`/`insert`, a map value, a constructor argument, a field default, a `return`, an annotated array or list literal (each element), and `I?` (a `Goblin` where a `Damageable?` is expected). An unannotated literal mixing types is refused: "give the list a type: `var foes: list[Damageable] = [...]`".
- **No other conversions:** an interface value doesn't convert to another interface or back to its concrete type (that is `is`, later).
- **Calling:** `d.take_damage(3)` calls the concrete type's method (dynamic dispatch). A `var self` method needs a changeable place, as for structs (`var foes`, a loop item of a `var` list, a `var` parameter). Arguments are checked against the interface's signature; labels work as for methods.
- **`pure` and `par for`:** a `pure` function and a `par for` body may call only interface methods declared `pure` (every implementation is then pure). In `par for f in foes`, `f.step(dt)` is allowed when `step` is declared `pure step(var self, dt: float)`, following the existing rule for `var self` methods on the loop's item.
- **No `==`, `!=`, map keys, `has` or `find`** on interface values: "interface values can't be compared; compare what a method returns".
- **`print`** of an interface value prints the concrete value (`Goblin(hp: 8)`).
- **Godot:** a `pub` function with an interface in its signature compiles but isn't callable from Godot.

## Values and memory

An interface value is two words: a pointer to the data and a pointer to the concrete type's method table.

- **Passed straight to a parameter (borrowed):** when the argument of a parameter of interface type is a concrete place (`show(g)` with `show(d: Drawable)`), the value points at the argument where it is: no copy, no allocation. Like a closure's stack environment, a borrowed value exists only as a call argument; storing or returning the parameter copies it (below).
- **`var` parameters:** `heal(var g)` with `heal(var d: Damageable)` passes `g` borrowed, so `var self` methods called through `d` change `g`. **A `var` parameter of interface type can't be assigned as a whole** ("can't replace `d`: it may be the caller's `Goblin`; change it through its methods"), because the caller's place may be a `Goblin` that a `Slime` doesn't fit in. Passing it on as `var` to another interface parameter is fine.
- **Stored (boxed):** everywhere else (a `let`, a field, a list item, a map value, a `return`, a constant), the concrete value is copied into the current arena and the interface value points at the copy. So storing follows the string rules: built in the current arena, `in x.mem { }` to store it somewhere longer-lived, refused out of `arena { }` with the existing messages. The escape check uses the coerced (interface) type, so a `Goblin` of plain numbers coerced to `Damageable` and stored out of `arena { }` is refused like a string.
- **Copying** an interface value (any store of a non-fresh one) copies the concrete value into the current arena through the table's copy entry, so copies are independent, as for every Kestrel value. A whole interface place (a field, a list item, a `var` local) can be assigned any implementing value.
- **Owning implementors** (a struct holding an `Arena` or a GPU object) may implement interfaces and may be passed borrowed, `var` included. They can't be stored as an interface: where the concrete type is known, it is a compile error ("`Sprite` owns a GPU texture, so it can't be stored as a `Drawable`; keep it in a `list[Sprite]`"); a copy of a borrowed one that reaches the table's copy entry (a callee storing its `Drawable` parameter) fails at run time (the message is under "Details settled by the spec review"). So interface values never own anything, and nothing needs dropping.
- **Closures:** a closure may capture an interface value (it gets a copy). Rule 7b doesn't look inside interface values.
- **Lists of interface values** work with every list method (`filter`, `map`, `sort_by` with a key, `remove b`, ...): items are 16-byte values, moved as bytes; `filter` copies through the copy entry.

## Details settled by the spec review

The Opus review of this spec and its plan (2026-10-19) found these; each is now part of the design.

- **Every parameter's `var` must match** in conformance, not only `self`'s: a `var` parameter is passed by pointer and a plain one by value, so the table call would use the wrong convention. "`Goblin.hit` doesn't match `Damageable.hit`: it should be `hit(var self, var n: int)`".
- **More conformance messages:** "`area` in `Shape` needs `self` or `var self` first"; "`Shape` declares `area` twice".
- **Coercion is shallow:** only a concrete value becomes an interface value. `list[Goblin]`, `[Goblin]`, `Goblin?` and maps of `Goblin` don't convert to their interface versions (existing mismatch message). A `var` argument coerces only from `T` to exactly `var I`: `var g` doesn't go to `var o: Damageable?` or `var list[Damageable]`, since the callee could write `none` or a `Slime` into it.
- **Only places are borrowed,** as KIR defines a place (a local, a parameter, a field, a list or array element; not a map lookup `m[k]` or a string index, which are boxed). An argument that is such a place is borrowed. Any other concrete argument (`show(Goblin())`, `show(make())`) is boxed. An owning one that isn't a place is refused: "`Pool()` owns an `Arena`; store it in a `let` first, then pass it".
- **A `var` interface argument is borrowed, never boxed,** from the first task that lowers interface values on: `heal(var g)` passes an interface value pointing at `g`, so changes land in `g`, never in a copy.
- **No whole assignment through any `var` interface parameter:** functions, methods, closures and function types alike.
- **Borrowed values after inlining:** the inliner turns a borrowed argument into a `Let`, which lives for the inlined body. `promote` and `scalarize` treat a borrowed `MakeIface` anywhere in a region like an `Arg::Ref` of its place's root (the place is pinned in memory, and a promoted region over it is left alone).
- **`array[T](n)` is refused when `T` holds an interface value** (it would fill the array with empty values that crash when used): "`array[Damageable](n)` would start with empty items; build it with a literal or `add`".
- **Natives can't take or return interface values:** "native functions can't take interface values".
- **Comparing:** a value that holds an interface value (directly or in a field) can't be compared with `==` (this uses its own "can't be compared" check, separate from rule 7b's). `d == none` on a `Damageable?` is allowed (it tests only whether there is a value), as for other optionals; the "can't be compared" check applies to comparing two values. `has`/`find` on a list of interface values give their existing "needs items that `==` can compare" message.
- **The owning run-time error** names the types but not the line, because copies happen deep inside generated copy functions that have no line: "can't copy a `Pool` into a `Damageable`: it owns an `Arena`; pass it as `var` or keep it in a `list[Pool]`" (a GPU object says "it owns a GPU object"). The copy entry is per (type, interface), so the second name is the interface, not a struct that holds it.
- **Interface method calls** reuse `check_method_call`'s receiver path (`receiver_root`, the `par for` write check, `map_loop_inside`, `check_changed_later`), and every place that matches `Res::Fn` for a method call (`changes_in`, the unused-result warning, KIR's method lowering) learns `Res::IfaceCall`.
- **A usability cost, stated:** in a method, `self.foes.add(Goblin())` boxes the goblin in the caller's arena, so it needs `in self.mem { }`, while `list[Goblin]` doesn't (the string rules).

## How it's built

Guidance for the plan; the builder may change details that don't change behaviour.

- **Checker:** `Ty::Iface(IfaceId)` (aggregate, `has_heap` true, not owning); `Tables::ifaces` with each interface's methods as abstract `FnInfo`s (no body, skipped by `check_fn`, so `check_args`, purity and the `par for` rules work unchanged); `StructInfo`/`EnumInfo` gain their interface lists; conformance is a pass after members are collected (`sig_accepts` is the model). Coercion goes in `BodyChecker::coerce` next to the `Fn` and optional branches, recorded in `Tables::coerce`; the optional branch learns `T` to `I?`. `region()` and the move check use the coerced type when there is one.
- **KIR:** `KTy::Iface` (16 bytes: data, table). `ExprKind::MakeIface { table, data: Borrowed(Place) | Boxed(Expr) }`, mirroring `MakeFn`'s stack and heap environments, with the same invariant (a borrowed `MakeIface` only as a direct call argument, checked by `check_stack_envs` or its sibling, and `owned()` turning borrowed into boxed). A call through an interface is `CallVirtual { recv: Place, slot, args, sig }`: it loads the method's address from the table and passes the data pointer where a method expects `self`. `fixup(KTy::Iface)` copies through the table's copy entry. A borrowed `MakeIface` takes a place's address, so `scalarize` pins it and `promote` treats it like an `Arg::Ref`.
- **The method table** (one per (type, interface) pair): a type id, the copy function, the print function, then the methods in declaration order. It is static data with function addresses (`declare_func_in_data` + `write_function_addr`, which both the JIT and object backends relocate). The copy function, per (type, interface), duplicates the value into the current arena and fixes up its heap parts (an owning type's copy entry is the run-time error above). Methods are the type's real methods: their ABI already takes `self` by pointer.
- **Devirtualising** (a `CallVirtual` whose `MakeIface` is visible after inlining becoming a direct call) is a follow-up, as for closures.

## Tests

Program tests (JIT and exe, under the poison mode): a mixed `list[Damageable]` with `var self` calls in a loop; a borrowed parameter and a `var` parameter changing the caller's struct; copies of interface values, lists of them, and structs holding them, changed independently; a value stored with `in x.mem { }`; `Damageable?`; a map of interface values; an enum implementing an interface; interface fields inside a struct stored in a list; `filter`/`map`/`sort_by` over interface lists; a closure capturing an interface value; `print`; an interface from another module (`cli/tests/modules.rs`); a borrowed owning implementor (`Sprite`-like struct holding an `Arena`) and the run-time copy error (`// expect-stderr:`/`// expect-exit:`); calls in a `par for` with a `pure` method. A memory test (`cli/tests/` in the `text_memory.rs` style): passing a struct to an interface parameter 100,000 times allocates nothing. Error tests for every message above.

**A measurement** in `kestrel/bench/kernels`: a loop calling a method through `list[Damageable]` against the same loop over `list[Goblin]` and against Rust's `Vec<Box<dyn Trait>>`, losses included.

## Docs

The syntax spec's sections and "Out of scope" line, the README, and this spec's "As built".

## Rulings made on the user's behalf

1. **Methods only, at least one, no default bodies.** Cost: a shared helper between implementations is a free function.
2. **A `pure` implementation satisfies a non-`pure` declaration.** Cost: none known.
3. **Interface values don't convert to each other or back.** Cost: until `is`, a value can't be narrowed.
4. **Values passed straight to an interface parameter are borrowed (no allocation); all other uses box a copy in the current arena.** Cost: storing an interface value allocates, like a string.
5. **A `var` interface parameter can't be assigned as a whole.** Cost: replacing the caller's value through an interface parameter needs a concrete type.
6. **Owning implementors can be borrowed but not stored; a copy that reaches the table fails at run time, without a line number.** Cost: one run-time error where a compile error isn't possible (a callee storing a borrowed parameter), and it doesn't say where.
6a. **`array[T](n)` with interface items is refused.** Cost: such arrays are built with a literal or `add`.
6b. **Non-place owning arguments to interface parameters are refused.** Cost: one `let` first.
7. **A `pure` function or `par for` body calls only `pure` interface methods.** Cost: an interface whose methods aren't declared `pure` can't be used in parallel code.
8. **Rule 7b doesn't look inside interface values.** Cost: a stored closure capturing an interface value whose concrete type holds closures can nest copies (accepted growth, as for ruling 7b of the closures spec).
9. **No devirtualising this round.** Cost: every interface call is an indirect call.

## As built

Built 2026-10-19 on `main` in five tasks (commits 313f10c, 4315415, 2029066, d6698c4 and 93b4e3e for the compiler; the measurement and docs are Task 5's commit, "Measure interface calls and document interfaces"; the final review's fixes are a0c311e, 876b727, d04d151, 590737f and the re-review's fix "Count a map get as reading its map's place"). Interfaces work as designed: declaring, implementing, interface-typed values everywhere the design lists, calls through method tables, borrowed and boxed values, copies through the table's copy entry, and the owning rule. Test counts as the tasks left them: 845 after Task 1, 860 after Task 2, 870 after Task 3 (full `cargo test --workspace` each time). Task 3's last refactor and Task 4 ran focused targets only (kir, codegen, check, runtime, the cli `errors`, `modules`, `iface_memory`, `text_memory`, `closure_memory` and `poison` targets, and the interface programs in JIT and exe), because the user's machine was under load; workspace clippy was clean after Task 4. The final fixes ran the same way (check, kir and driver unit tests, the cli `errors` and `iface_memory` targets, and the new and changed programs in JIT and exe), with workspace clippy clean at the end. **Owed:** one full `cargo test --workspace` run, the whole `programs` target and the Godot clippy (`kestrel/godot`) since Task 3. The measurement is in `kestrel/bench/kernels/results.md` ("Interface calls"): a call through `list[Damageable]` takes about 5.2 ns, 7-11 times a direct call and 14-19% more than Rust's `Box<dyn>`. What differs from the design, or was added to it:

**Declarations**
- **Interface methods are `FnDecl`s with an empty body** (like natives); `self` is typed as the interface. Each gets an abstract `FnInfo` (`is_abstract`) that `check_fn` skips and KIR lowers as a stub whose body is `Stmt::Trap` ("an interface method with no body was called"). The stubs are not exported to Godot. Their `FnId`s sit at the interface's position among the items, as a struct's methods do.
- **A bad member is reported and left out of the interface.** "An interface needs at least one method", a missing `self` and a duplicate name are checker errors, not parse errors, so they show with the other checker errors. Parameter names don't have to match in conformance, only types and `var`s.
- **Name clashes:** an interface and a struct with the same name give the existing "`X` is defined twice"; `resolve_type` prefers a struct, then an enum, then an interface.
- **The design's first example was invalid** (a field `hp` and a method `hp` can't share a name); the examples use `health`.

**Checker**
- **`Res::IfaceCall { iface, slot }`** is the resolution of a method call on an interface value (`slot` is the declaration index). `Tables::callee(res)` gives the abstract `FnInfo` for it or for a plain `Fn`, which is what `changes_in`, the unused-result warning and the argument checks use. Interface methods reuse `check_method_call`'s receiver path, so a `var self` call needs a changeable place and a `par for` body may call only `pure` ones.
- **Lent arguments are recorded** in `Tables::iface_borrows` (each also has a `coerce` entry to the interface); every other concrete-to-interface `coerce` entry is a box. `is_kir_place` is the checker's copy of KIR's `place()` and of `hoisted`; if either changes, change it too. If KIR disagrees (it finds no place for a lent argument) it boxes, which costs an allocation and never crashes in a release build (a debug build asserts).
- **The mixed-literal message** is given at the second item that differs, only when the two types share an interface; otherwise the plain mismatch stays.
- **`array[T](n)` is refused more widely than "`T` is an interface":** also when `T` is a struct, enum or array that holds one as a field, payload or element (`zero_fill_breaks`). That also refuses `array[[Damageable]](n)`, which would be fine because an empty inner array holds no interface value (a follow-up).
- **`d == none`** is not a comparison of interface values (the check is skipped when a side is the `none` literal), but the language has no `x == none` for any optional ("can't tell what kind of `none` this is"), so the design's "allowed" only means that the interface-comparison check doesn't refuse it.
- **Natives** that take or return an interface value (or a type holding one) are refused with "native functions can't take interface values". A user file can't declare a native, so this is tested only in checker unit tests.
- **The "can't replace" example type** is the first implementor other than the assigned value's type, not always `Goblin`. The owning-type messages say "an `Arena`" or "a GPU object" (the design's example said "a GPU texture").
- **A built-in method's receiver** is checked against a later argument that changes its root, like a `var self` user method, when the built-in takes the receiver before the arguments and reads through it after: every string method (the receiver is a pointer and length), and a list's `has`, `find`, `join`, `filter`, `map`, `any`, `all` and `sort_by` (they take the list's header). `vs[0].name.find(drop_first(var vs))` is "`vs` is passed here, but a later argument changes it". That hole already existed for strings; the review of Task 3 found it by way of interface receivers. `add`, `insert`, `remove_at` and the map methods evaluate their arguments before they find the receiver's place, so `self.ids.add(self.alloc())` and `q.add(take(var q))` are accepted (Task 4 refused them for every list and map method; the final review narrowed it, `builtin_receiver_args.kes`).
- **An `if` or `match` of places passed as an argument or receiver** is kept shallowly (a header that still points at the arm's place), so `check_changed_later` looks through the arms' tails, and through a field or item of such a value, for every argument and receiver whose type holds heap memory: `use2(if c { ws[0].ds[0] } else { h }, drop_first(var ws))` is refused like the place itself. Before, it read freed memory, for strings as well as interface values (the final review's Critical). A map `get` of a value that holds heap memory counts as reading the map's place too, since KIR reads the entry shallowly (its default is copied in its turn): without that, removing the receiver copy (below) let `ws[0].m.get("a", hd).plus(drop_first(var ws))` read freed memory (the re-review). It refuses a `get` from a borrowed map, whose value KIR does copy (a follow-up). The same rule, `check_kept_parts`, now covers the values that are kept until the last one has run and weren't checked at all: `print`'s arguments (`print(ws[0].ns[0], drop_first(var ws))`), the sides of an operator and the parts of an interpolated string, which say "`ws` is read here, but a later part of this expression changes it" (`changed_later_tails.kes`).

**KIR and codegen**
- **`KTy::Iface`** is a unit type (16 bytes: data pointer, table pointer), like `KTy::Fn`. It carries no interface id, so `list[A]` and `list[B]` share KIR helpers.
- **`MakeIface { table, data: Borrowed(Place) | Boxed(Expr) }`** mirrors `MakeFn`. `check_stack_envs` holds the invariant: a borrowed `MakeIface` appears only as a direct argument of `Call`, `CallIndirect` or `CallVirtual`, and `owned()` turns it into a boxed one. `scalarize` pins the place of a borrowed `MakeIface` and the receiver of a `CallVirtual`, and `promote` leaves a region alone when one mentions its root.
- **`CallVirtual { recv: Place, slot, args, sig }`** (an expression and a statement) reads the data pointer and the method address before evaluating the arguments and passes the data pointer where a method expects `self`. `sig` excludes `self`.
- **Method tables** (`Module::tables`, one per (concrete type, interface)) are `[type_id, copy, print, method_0, ...]`, 8 bytes each, in read-only data, with the function addresses relocated by `declare_func_in_data` and `write_function_addr` in the JIT and in objects. Type ids are 1, 2, ... per concrete type in order of first use. The print entry is one function per concrete type (`__kestrel_iface_print_<n>`) and the copy entry one per table (`__kestrel_iface_copy_<n>`).
- **The copy entry** duplicates the value into the current arena (`ExprKind::BlockDup`, which calls `kestrel_rt_dup` with a constant size and alignment) and then runs `fixup` on the copy for the heap parts it holds. For an owning type it is `Stmt::Fail(msg)` instead, which calls the new runtime function `kestrel_rt_fail(msg, len)`: the run-time error has no line number and exits with 101. It is a new statement and not a new `PANIC_*` kind because the message carries type names.
- **Copying** an interface value is `fixup(KTy::Iface)`, which emits `Stmt::CopyIface(place)`: call the table's copy entry and store the new data pointer.
- **A non-place interface receiver** (an `if` or `match` of places, or a field of one) is used as it is, like a place. Task 4 deep-copied it through the copy entry before the call (`own_iface_receiver`), an allocation per call, so that an argument freeing the original's arena couldn't leave the call reading freed memory; the final fix removed that copy, because the checker now refuses a later argument that changes the places such a receiver is read from (see "Checker").
- **Interface constants are built once**, before `main`, and read in place, like constants holding a collection: `Tables::const_read_in_place` (which KIR's `hoisted` and the checker's `is_kir_place` both ask) includes types that hold an interface value. Before, each use built the constant again and boxed it in the current arena, so `DEFAULT.think()` in a per-frame loop grew the program arena without bound (the final review; `iface_memory.rs` now measures it). A copy of one (a variable, a returned value, a list item) is its own (`iface_consts.kes`).
- **A lent `var` argument whose place KIR can't find** is an internal error (the existing `expect` in `call_parts`), never a box, because a box would change a copy where the caller's own value was meant.
- **`print`** of an interface value calls the table's print entry with the data pointer, so `Barrel.Full(oil: 3)` prints as the enum does.
- **Mutations that were reverted:** boxing every lent argument fails `iface_memory`; dropping `fixup` from the copy entry prints freed-list garbage under poison (`iface_copies.kes`); dropping the deep copy of a non-place receiver printed a wrong number (since replaced by the checker's refusal, held by `changed_later_tails.kes`). The `promote` rule for a borrowed place shows in no program's output (the mutant still printed the right numbers) and is held by a KIR test only.

**Tests.** Program tests (JIT and exe, poisoned): `ifaces`, `iface_args`, `iface_decl`, `iface_copies`, `iface_lists`, `iface_owning` (with `// expect-stderr:` and `// expect-exit: 101`), `iface_promote` (a loop element lent and changed through `var`), `iface_enum_copy` (an enum implementor's copies), `iface_consts`, `builtin_receiver_args` and `map_get_default`. Error tests: `iface_decl`, `iface_use`, `builtin_receiver_changed` and `changed_later_tails`. Unit tests in `check/tests/iface*.rs`, `kir/tests/ifaces.rs`, `codegen/tests/ifaces.rs` and `syntax/tests/parser_iface.rs`; `cli/tests/modules.rs` (an interface from another module, run and built); `cli/tests/iface_memory.rs` (passing a struct to an interface parameter 100,000 times, or calling an interface constant's method 100,000 times, allocates nothing); and `driver/tests/builtin.rs` (the copy entry of a type holding a `draw.Texture` says "it owns a GPU object"). `is` tests are not built.

**Rulings after the design** (the controller's, on the user's behalf): the owning run-time copy error names the interface as its second name, and the design's example is changed to match (Task 4); `d == none` is as above (Task 2); a lent argument that KIR can't find a place for is boxed (Task 3); a non-place interface receiver is deep-copied through the copy entry (Task 4), later replaced by the checker refusing the shape for every argument and receiver (final review); a built-in method's receiver is checked against later arguments (Task 4), narrowed to the built-ins that read through the receiver after the arguments (final review); and interface constants are hoisted (final review).

**Open items** are in `docs/superpowers/plans/2026-10-09-core-2a-followups.md`, "Found while building interfaces (2026-10-19)".
