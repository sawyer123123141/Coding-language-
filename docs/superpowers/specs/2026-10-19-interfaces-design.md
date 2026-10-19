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
    hp: int = 10
    take_damage(var self, amount: int) { self.hp -= amount }
    pure hp(self) -> int { self.hp }
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
- **No `==`, `!=`, map keys, `has` or `find`** on interface values: "interface values can't be compared".
- **`print`** of an interface value prints the concrete value (`Goblin(hp: 8)`).
- **Godot:** a `pub` function with an interface in its signature compiles but isn't callable from Godot.

## Values and memory

An interface value is two words: a pointer to the data and a pointer to the concrete type's method table.

- **Passed straight to a parameter (borrowed):** when the argument of a parameter of interface type is a concrete value (`show(g)` with `show(d: Drawable)`), the value points at the argument where it is: no copy, no allocation. Like a closure's stack environment, a borrowed value exists only as a call argument; storing or returning the parameter copies it (below).
- **`var` parameters:** `heal(var g)` with `heal(var d: Damageable)` passes `g` borrowed, so `var self` methods called through `d` change `g`. **A `var` parameter of interface type can't be assigned as a whole** ("can't replace `d`: it may be the caller's `Goblin`; change it through its methods"), because the caller's place may be a `Goblin` that a `Slime` doesn't fit in. Passing it on as `var` to another interface parameter is fine.
- **Stored (boxed):** everywhere else (a `let`, a field, a list item, a map value, a `return`, a constant), the concrete value is copied into the current arena and the interface value points at the copy. So storing follows the string rules: built in the current arena, `in x.mem { }` to store it somewhere longer-lived, refused out of `arena { }` with the existing messages. The escape check uses the coerced (interface) type, so a `Goblin` of plain numbers coerced to `Damageable` and stored out of `arena { }` is refused like a string.
- **Copying** an interface value (any store of a non-fresh one) copies the concrete value into the current arena through the table's copy entry, so copies are independent, as for every Kestrel value. A whole interface place (a field, a list item, a `var` local) can be assigned any implementing value.
- **Owning implementors** (a struct holding an `Arena` or a GPU object) may implement interfaces and may be passed borrowed, `var` included. They can't be stored as an interface: where the concrete type is known, it is a compile error ("`Sprite` owns a GPU texture, so it can't be stored as a `Drawable`; keep it in a `list[Sprite]`"); a copy of a borrowed one that reaches the table's copy entry (a callee storing its `Drawable` parameter) fails at run time: "can't copy a `Sprite` into a `Drawable` on line N of f.kes: it owns a GPU texture". So interface values never own anything, and nothing needs dropping.
- **Closures:** a closure may capture an interface value (it gets a copy). Rule 7b doesn't look inside interface values.
- **Lists of interface values** work with every list method (`filter`, `map`, `sort_by` with a key, `remove b`, ...): items are 16-byte values, moved as bytes; `filter` copies through the copy entry.

## How it's built

Guidance for the plan; the builder may change details that don't change behaviour.

- **Checker:** `Ty::Iface(IfaceId)` (aggregate, `has_heap` true, not owning); `Tables::ifaces` with each interface's methods as abstract `FnInfo`s (no body, skipped by `check_fn`, so `check_args`, purity and the `par for` rules work unchanged); `StructInfo`/`EnumInfo` gain their interface lists; conformance is a pass after members are collected (`sig_accepts` is the model). Coercion goes in `BodyChecker::coerce` next to the `Fn` and optional branches, recorded in `Tables::coerce`; the optional branch learns `T` to `I?`. `region()` and the move check use the coerced type when there is one.
- **KIR:** `KTy::Iface` (16 bytes: data, table). `ExprKind::MakeIface { table, data: Borrowed(Place) | Boxed(Expr) }`, mirroring `MakeFn`'s stack and heap environments, with the same invariant (a borrowed `MakeIface` only as a direct call argument, checked by `check_stack_envs` or its sibling, and `owned()` turning borrowed into boxed). A call through an interface is `CallVirtual { recv: Place, slot, args, sig }`: it loads the method's address from the table and passes the data pointer where a method expects `self`. `fixup(KTy::Iface)` copies through the table's copy entry. A borrowed `MakeIface` takes a place's address, so `scalarize` pins it and `promote` treats it like an `Arg::Ref`.
- **The method table** (one per (type, interface) pair): a type id, the copy function, the print function, then the methods in declaration order. It is static data with function addresses (`declare_func_in_data` + `write_function_addr`, which both the JIT and object backends relocate). The copy function, per implementing type, duplicates the value into the current arena and fixes up its heap parts (an owning type's copy entry is the run-time error above). Methods are the type's real methods: their ABI already takes `self` by pointer.
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
6. **Owning implementors can be borrowed but not stored; a copy that reaches the table fails at run time.** Cost: one run-time error where a compile error isn't possible (a callee storing a borrowed parameter).
7. **A `pure` function or `par for` body calls only `pure` interface methods.** Cost: an interface whose methods aren't declared `pure` can't be used in parallel code.
8. **Rule 7b doesn't look inside interface values.** Cost: a stored closure capturing an interface value whose concrete type holds closures can nest copies (accepted growth, as for ruling 7b of the closures spec).
9. **No devirtualising this round.** Cost: every interface call is an indirect call.
