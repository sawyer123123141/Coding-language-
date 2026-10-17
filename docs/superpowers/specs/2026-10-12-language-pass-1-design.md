# Language pass, part 1: friction fixes (design)

Status: decided by Claude under the user's delegation. On 2026-10-12 the user agreed that the language, not the backend, should come next ("those sound good if u agree we shuold start them"), then said "review urself and double check thats the right thing to start. if u think it is then go." Architecture is delegated to Claude; syntax is the user's. Nothing here adds syntax: every change makes code that is rejected today compile, or makes a message clearer. The user may veto any ruling (listed at the end).

## Why

Kestrel's speed is its strongest part (the particle step is at 1.15x Rust); its everyday language is its weakest. This pass fixes friction in features that already exist and need no syntax decision. What comes after it needs the user's picks: list and string basics (lists have only `add` and `.len`; strings have no length, indexing or joining), then interfaces and closures, with the `@` annotation design written alongside.

Sources: the follow-ups file (`docs/superpowers/plans/2026-10-09-core-2a-followups.md`), the user's first test program (a Breakout game, 2026-10-12), and four read-only surveys of the checker, KIR and runtime.

One follow-up turned out stale: "a `pure` function can't call a helper that only changes a `var` argument rooted at its own local". That has worked since 4073379 (2026-10-09, "Let pure functions take var parameters") when the helper is `pure`, and `tests/programs/pure_var.kes` covers it. The note was written earlier the same day. The voxel demo's `Builder` comment (`demos/voxels/world.kes:188-191`) repeats the stale belief. Section F only corrects the notes.

## What changes

Six parts, A to F. A and B change the checker's ownership rules (B also changes KIR for `let ... else`). C and D change what a construction may be. E carries file names to runtime errors. F is notes.

### A. Moves follow the program's paths

An owning value (one that holds an `Arena` or a GPU object: `Texture`, `Mesh`, a struct with an `Arena` field) moves instead of copying. Today the checker keeps one set of moved variables for the whole function (`BodyChecker::moved`, `check/src/body/regions.rs` `consume`). Once a variable is moved anywhere, every later line that names it is an error, whichever path it is on, and moving a variable declared outside a loop is always an error. So these are rejected today and compile after this part:

```
if c { a.add(t) } else { b.add(t) }              // each path moves `t` once

if bad { keep.add(t); return }
use_it(t)                                        // the move's path left the function

for slot in slots {
    if slot.free { slot_list.add(t); break }     // the move's path leaves the loop
}

var cur = make()
while running {
    done.add(cur)
    cur = make()                                 // `cur` has a value again before the loop goes around
}
```

The checker tracks, at each point of a function, which owning locals may have been moved, where (the span of a move), and whether every path to that point moved them:
- **Branches.** Each branch of an `if` or `match` starts from the state before it. After it, a local is moved if some branch that can finish moved it. A branch that always leaves (`return`, `break`, `continue`, or a nested `if` whose branches all do) adds nothing. An `if` with no `else` counts its missing branch as one that moves nothing. The right side of `and`/`or` runs only sometimes, so its moves count as "may have been moved" afterwards.
- **Loops.** A loop body starts from the state at the loop's start (for `while`, before its condition, since the condition runs on every turn). At the end of the body and at each `continue`, a local declared outside the loop that is moved now, but wasn't when the loop started, is an error at its move. The message is today's: "`t` is moved inside a loop, so the next time around it would already be gone". The help becomes "declare it inside the loop, give it a new value before the loop goes around, or leave the loop (`break` or `return`) right after moving it". After the loop, a local is moved if it was moved at the loop's start or at any `break`.
- **Assignment gives a value back.** `t = value` (plain `=`, the whole variable) makes `t` usable again after the statement. The assignment's target isn't a use, so it doesn't report "moved earlier". The value is checked first, so `t = f(t)` on a moved `t` is still an error. Assigning a part (`t.x = 1`, `t.items[0] = ...`) of a moved variable is still an error.
- **`let x = opt else { ... }`.** The `else` block sees the state before the unwrap moves `opt`: there, `opt` is `none` and wasn't consumed. Today using `opt` inside the `else` block reports "moved earlier".
- **`par for`.** Unchanged in effect: a body can't `break` or `return`, so moving an outer local in it is always the loop error, and the body's own write rules apply as before.

Messages for using a moved local:
- Moved on every path to the use: today's "`t` was moved earlier, so it can't be used here".
- Moved on some paths: "`t` may have been moved earlier, so it can't be used here", with help "it's moved on some paths to this line; give it a new value first". (The checker has spans but no line numbers, and diagnostics have no secondary labels yet, so the message can't point at the move.)
- A value an `if` or `match` expression moves out of one of its branches (`let y = if c { a } else { b }`) counts as moved on some paths, for each of `a` and `b`.

**KIR and codegen don't change.** A move already zeroes its source (`codegen/src/func/arenas.rs` `move_into`), and every drop skips a zeroed value: `kestrel_rt_arena_free` returns on null, `kestrel_rt_gpu_drop` ignores id 0, and an optional's drop tests its tag. So a value moved on one path and not the other is dropped exactly once at the end of its scope, and a re-assigned local drops its new value. This part only stops the checker from rejecting programs that were already safe to run; it must never accept a use of a possibly-moved local, because that would read zeroed memory (a null arena).

The state lives in a new `check/src/body/moves.rs`, which also takes `consume` out of `regions.rs` (396 lines now). The four loop forms (`while`, `for .. from .. to`, `for .. in` over arrays and lists, `for .. in` over maps) share one helper that runs a body and does the loop's checks.

### B. Views: nothing moves what a view reads; `let ... else` reads in place

A view is a name for a place that the program reads without copying: a `for` loop's item and a `match` arm's binding.

**B1. A gap today: moving a variable while a `match` arm reads it.** `match_guards` stop assigning to, or passing `var`, the matched place while an arm's binding is live (`check_not_matched`), but `consume` never looks at them. So `match c { V b { world.add(c); print(b.f) } }` compiles, and `b` then reads `c` after the move zeroed it. With an owning field, that can be a null arena. After this part, moving the matched variable (or anything holding it) while an arm's binding is live is an error: "can't move `c` here: this `match` arm's `b` reads from it", with help "move it after the `match`, or copy what you need out of `b` first".

**B2. `let x = opt else { ... }` reads in place when it can.** Today it always moves or copies the value inside the optional (`check_let_else` in `check/src/body/collections.rs`, `let_else` in `kir/src/lower/collections.rs`). For an owning payload that means unwrapping a parameter or a field is an error: `let t = self.tex else { return }` fails with "can't move this out: it holds a GPU object, so it can't be copied, and only whole variables move", and `let c = chunk else { return }` with a `Chunk?` parameter fails with "can't move parameter `chunk`". For a string or list payload it means a deep copy.

`let x = opt else` becomes a view of the optional's payload when all of these hold:
1. It is `let`, not `var`. `var x = opt else` stays an independent copy (`tests/programs/constants_hoisted_stores.kes` relies on it).
2. `opt` is a place: a local, a parameter, `self`, or fields and list or array elements of one. Not a map lookup (`m[k]`, which isn't a place in KIR), not a call or an `if`, and not a `T` that the checker turned into a `T?`.
3. The place's root can't change: a `let` local, a parameter that isn't `var`, or `self` in a method that takes `self`. That keeps writes from changing what the view reads, with no new guards.
4. The payload holds heap memory or owns something. Numbers, bools and vectors stay copies, which are free and keep `scalarize` and `promote` working on them.
5. It isn't a whole owning local that could simply move (an `Ident` that isn't a parameter or a view). That case keeps today's move, so `let x = c else { ... }` followed by `world.add(x)` still works (`tests/programs/optionals.kes`, `take`).

A let-else view is like a loop item: uses of `x` read the payload in place, storing `x` somewhere copies it (or, for an owning payload, is an error, below), and nothing drops `x` (its owner drops the whole optional). While a let-else view is in scope (to the end of its block), moving its root is an error: "can't move `c` here: `x` reads from it", with help "move it after the block that declares `x`". Moving an owning view is an error that names where it reads from: "can't move `x`: it's inside `c`, and it holds a GPU object, so it can't be copied" (for a parameter root, "it's inside parameter `c`, which belongs to the caller, ...").

The checker records each let-else view (a new `Tables` set keyed by the binding, like `moves` and `locked_map_loops`), and KIR follows it: it takes the place (`place_or_temp`, which spills a non-literal index once), emits `if tag(place) == 0 { else block }`, and maps `x` to the payload place through `elems`, with no `Let` and no `declared`. KIR treats a recorded view whose value isn't a place as a compiler bug (`expect`), because a fallback to a copy would drop an owning payload twice.

The checker code moves to a new `check/src/body/let_else.rs` (it doesn't belong in the arrays-and-lists file), and the KIR side gets a small helper beside `let_else`.

### C. Field defaults and constants take struct and enum values

Today a field default may build only `vec2` and `vec3` (`check/src/body/consts.rs` `is_const_expr` with `user_structs = false`) and can't name a constant whose type holds a user struct or an enum (`uses_struct_constant`). Neither defaults nor constants may be enum values (`State.Idle` is a field access, `Shape.Circle(1)` a method call). So `tint: Color = draw.RED`, `state: State = State.Idle`, `let START = State.Idle` and `inventory: Inventory = Inventory()` are all rejected. The ban came from two bugs in 149a32f: a default built in an `arena { }` that dangled (the region check didn't see the default's memory), and a compiler stack overflow on a default that builds its own struct (`kids: list[Node] = [Node()]`).

After this part:
- **C1. Plain values.** A default or constant may build any struct or enum value (`Color(1, 0, 0)`, `State.Idle`, `Shape.Circle(1)`) and name any constant, as long as the expression is otherwise a constant expression (literals, operators, constants, pure built-ins, array and map literals). `uses_struct_constant` goes away. A value with no heap memory can't dangle, and a plain struct can't contain itself (`check_self_containment` already rejects that), so these need nothing more.
- **C2. Values with heap memory.** A default may build a struct that holds strings, lists, arrays or maps (`inventory: Inventory = Inventory()`). The dangling half of 149a32f stays fixed by `default_allocates`, which is already true for every heap-typed default except `none`, a string literal or constant, and an empty array; the region check then sees the construction's memory.
- **C3. A cycle check.** Defaults and constants that build each other forever are an error, found before KIR: a new `check/src/body/default_cycles.rs` walks a graph whose nodes are field defaults and constants. A construction with omitted fields points at those fields' defaults (a field given at the construction breaks the edge, so `[Node(kids: [])]` is fine), and a constant's name points at the constant. A loop is reported once, at the default or constant that closes it, in the style of the constants' loop message: "field defaults build each other in a loop: Tree.kids -> KIDS -> Tree.kids; set the field when you build it, or start it empty (`[]`)". It runs after `check_field_defaults`, and only when there are no earlier errors (it needs `ctor_args`). Direct and optional cycles (`A { b: B = B() }`, `B { a: A = A() }`) are still "contains itself" errors from collection.
- **Owning values stay out.** A default may not build a value that owns an `Arena` or a GPU object, except `Arena()` itself (allowed today): "field defaults can't build a value that owns an `Arena` or a GPU object; set the field when you build it". This is untested ground, so it stays closed.
- **`default_allocates` looks through optionals.** `name: string? = NAME` is wrongly treated as allocating today (the type is `string?`, not `string`), so building the struct inside an `arena { }` and storing it outside is a false error. The function compares the payload type for an optional. (The survey also suspected `xs: list[int]? = []`, but `[]` can't be a `list[int]?` today: "can't tell what `[]` holds".)

The remaining rejection reads "field defaults can only use literals, operators, constants, struct and enum values, and pure built-ins like `sqrt`" (and the same for constants).

`tests/errors/field_defaults.kes` changes accordingly: `inner: Inner = Inner()` compiles, `kids: list[Node] = [Node()]` and `kids: [Tree] = KIDS` (with `let KIDS = [Tree()]`) are cycle errors, and the arena error on line 27 stays.

### D. `vec2()` and `vec3()` are zero; clearer construction errors

From the user's Breakout test: `let velocity : vec3()`, `vec3()`, and `Ball(...)` with no `struct Ball`.

- **D1.** `vec2()` and `vec3()` with no arguments are zero vectors, like Godot's `Vector2()` and `Vector3()`. That works anywhere a construction does, defaults and constants included. Giving some components but not all (`vec3(1)`, `vec3(x: 1)`) is still an error, with the help "give every component, or none for a zero vector: `vec3()`". The checker accepts an empty argument list for the two built-in vector structs, and KIR's `construct` fills a built-in struct's missing fields with `0.0`.
- **D2.** A construction missing several fields reports one error: "missing fields `y` and `z` for `vec3`" (one missing field keeps today's "missing field `x` for `P`").
- **D3.** A type followed by `(` where a `let` or `var` expects `=` (`let velocity: vec3()`) reports "a type goes after `:`, and the value after `=`", with help "write `let velocity: vec3 = vec3()`" (the statement's own keyword, name and type). The same applies to a struct field (`velocity: vec3()`), with help "write `velocity: vec3 = vec3()`". Today both report "expected `=`, found `(`". The help is given only when the type is a plain or generic name (`vec3`, `list[int]`), the forms that can be built by calling them.
- **D4.** Calling a capitalized name that is neither a struct nor a function (`Ball(pos: ...)` with no `struct Ball`) reports "unknown struct `Ball`", with help "declare it at the top level of a file (`struct Ball { ... }`), or bring it in from a module with `use { Ball from m }`". Today it reports "unknown function `Ball`", which sent the user (2026-10-12) looking for classes. A lowercase name keeps "unknown function `f`".

### E. Runtime errors name the file

Runtime errors give a line but not a file (`kestrel_rt_panic(kind, line)` and its siblings in `runtime/src/panic.rs`, `arena.rs` and `map.rs`), so in a program with modules, "on line 24" doesn't say which file. After this part every runtime error names its file when the compiler knows it:
- "index 5 out of bounds for length 2 on line 24 of geom.kes"
- "can't add a key to a map while looping over it (line 13 of game.kes); ..."
- a native's error: "draw.circle was called before window.open on line 7 of shapes.kes"
- in the Godot add-on: "Kestrel: index 3 out of bounds for length 2 on line 24 of world.kes (in `crash`)".

The name is the file's base name, never a full path (a built program mustn't carry the build machine's folders). A built-in module's file is `draw.kes` and so on.

How the file travels:
- **KIR.** A KIR line is a `u32`, set in one place (`FnLowerer::line`, `kir/src/lower/mod.rs`) and copied everywhere else. It now packs the file too: the file's index (its position in the driver's file list, from a new `LineIndex::file`) in bits 21-30 and the line in bits 0-20, with bit 31 clear. The entry file is index 0, so a single-file line is unchanged. A line past 2,097,151 is clamped; a file index past 1,022 packs as 1,023, which means "no name". The inliner already keeps each statement's own line, so a check inlined from another module still names that module.
- **The module.** `kir::Module` gets `files: Vec<String>` (base names, by index), filled by the driver after lowering. Tests that build a `LineIndex` from one string have no names.
- **Codegen.** A helper splits a packed line into the line and a pointer to the file's name: static data emitted once per file, null when there is no name. Every call to a runtime error function passes the name pointer as one more argument (`panic`, `panic_index`, `alloc`, `map_insert`). `SetLine` also stores the pointer, in a second static beside `kestrel_rt_call_line`, for natives.
- **Runtime.** The functions take the extra argument and add " of NAME" after the line when it isn't null.

Passing the name with each error, instead of registering a table in the runtime, keeps it right when one process holds several programs (the Godot add-on loads four in its test, and hot reload replaces one) and on `par for` threads, which format the message where the error happens.

The two messages written "(line N)" become "(line N of NAME)". Every other existing message keeps "on line N" as its start, so the tests that look for those words still pass.

### F. Stale notes

- The follow-ups entry about `pure` helpers is marked resolved (since 4073379), and `demos/voxels/world.kes`'s comment stops claiming the gap (the `Builder` stays: it reads like Godot's `SurfaceTool`).
- The follow-ups entries for B2 (`let ... else` always moves), C (struct-valued defaults) and E (file names) are marked done; the handoff's ruling J and the window spec's field-default line are updated.

## Testing

Every part is test-first: a failing test, then the change.
- **A:** checker tests in a new `check/tests/moves.rs` (each example above compiles; a use after a move on one branch gets the "may have been moved" message; a move in a loop with neither `break` nor a new value is the loop error; `continue` after a move is the loop error; a move in a `while` condition is the loop error; `t.x = 1` after a move is an error; using `opt` in a let-else's `else` block compiles). A program test that runs both branches of a conditional move and counts drops through the GPU drop counter in `codegen/tests/gpu.rs` (exactly once each). `tests/errors/moves.kes` for the messages.
- **B:** checker tests for B1 (moving the matched variable in an arm with a live binding) and B2 (views over a parameter, a field, a loop item of a `let` list; a `var` let-else still copies; a whole owning local still moves; moving a view's root and moving an owning view are errors). Drop-count cases in `codegen/tests/gpu.rs` for let-else views over a parameter and a field (dropped once, by the owner). A program test that unwraps an optional texture field in a method (`tests/programs/let_else_views.kes`, headless). The existing `optionals.kes`, `par_for_strings.kes`, `promote_exits.kes` and `scalarize_payloads.kes` keep their output.
- **C:** a new `check/tests/defaults.rs` (enum and struct values in defaults and constants; a heap struct default; the cycle errors, including through a constant; an owning default; `string?` and `list[int]?` defaults in an `arena { }`), the updated `tests/errors/field_defaults.kes`, and a program test that builds structs with those defaults and prints them.
- **D:** a program test printing `vec2()` and `vec3()` (in a local, a default and a constant), and error tests for a partial vector, several missing fields, the two `: T()` forms, and an undeclared capitalized constructor.
- **E:** `cli/tests/modules.rs`'s runtime-error test asserts "on line 24 of geom.kes" and gains an exe variant; a native error from a module file names it; `map_loop_add.kes` and the arena "(line N)" pin are updated; codegen and KIR unit tests (one unnamed file) keep their messages.
- **Everything:** `cargo test --workspace --no-fail-fast` and clippy (`-D warnings`) in `kestrel/` and `kestrel/godot`; the Godot add-on rebuilt (`kestrel/godot/build.ps1`, Godot closed) and its headless test run (30 checks), because E changes the runtime's error functions.

## Not in this design

- List and string basics: they need the user's method names. Proposed next.
- `var velocity: vec3` with no value (starting at zero, like GDScript): a syntax question put to the user on 2026-10-12.
- Views over a `var` root: they would need guards until the end of the block and on every write.
- Calling a helper that isn't marked `pure` from a `pure` function: `pure` is a checked declaration by design (the syntax spec), so the checker won't infer it.
- Moving a part of a variable (`a.tex`): only whole variables move, as today.
- Retained-mode drawing (shapes that redraw themselves when changed): the user asked about it on 2026-10-12, and Claude recommended against it; a `draw.fill(rect, color)` shorthand is a naming question for the list and string proposal.

## Rulings made on the user's behalf

What each costs if wrong:
1. **Strings and lists wait; this pass goes first.** Both need method names from the user, and these fixes don't. Cost: the user's Breakout has no list removal until the next part (an `alive` flag works meanwhile).
2. **A use of a possibly-moved local is an error**, with its own message, rather than a runtime check. The message doesn't name the move's line (the checker has no line numbers). Cost: a program that knows a value is there on a path the checker can't see must restructure; finding the move is up to the reader.
3. **Assignment gives a moved `var` local its value back.** Cost: none known; the drop of the old value already skips a zeroed one.
4. **The loop rule checks the back edge** (the end of the body and `continue`), not the move. Cost: the error is reported at the move but found later, so a program with several moves in one loop gets one error per local.
5. **`let ... else` views need a root that can't change** (`let`, a non-`var` parameter, a `self` method), and a whole owning local still moves. Cost: unwrapping a field of a `var` local still copies (or, if owning, is still an error).
6. **Moving a view's root while the view is in scope is an error until the end of the block**, not until the view's last use. Cost: a program must move the root after the block, or restructure.
7. **Defaults may build heap-holding structs, with a cycle check**, rather than only plain ones. Cost: a new graph check; a cycle message the user hasn't seen.
8. **Defaults can't build owning values** (except `Arena()`). Cost: `pool: Pool = Pool()` where `Pool` holds an `Arena` must be set at construction.
9. **`vec2()` and `vec3()` are zero; a partial vector is still an error.** Cost: `vec3(1)` doesn't mean `vec3(1, 0, 0)`.
10. **Runtime errors name the file by its base name, "on line N of NAME".** Cost: two message shapes ("(line N)") change.
11. **The file travels packed into KIR's line** (10 bits of file, 21 of line), and the name pointer is passed to each runtime error call. Cost: one extra argument on four cold runtime calls and one extra store per native call; a program with over 1,022 files or a line past 2,097,151 loses the name or the exact line.
12. **The voxel demo keeps its `Builder`**; only its comment changes. Cost: none.
13. **"unknown struct" goes by the first letter.** A call to an unknown name that starts with a capital letter is reported as a missing struct. Cost: a capitalized function name that doesn't exist gets the struct message (Kestrel's functions are lowercase by convention); `GpuHandle()` in a user file now reads "unknown struct `GpuHandle`".

## As built (2026-10-12)

Built in six tasks, run subagent-driven on `main` the same day: the spec (a7bf139), the plan (2c3307e, 21f6079), then A (bcdfc4b, review fix fdf2b14), B (531b453, review fix 864ca01), C (0a47ed6), D (8dc9d3b), E (f94de78) and F (the notes commit after them). A final whole-pass review asked for one more round of fixes (2026-10-17, the commits after 31aaee9), which also added the depth limit below. Tests went from 554 to 602 (`cargo test --workspace --no-fail-fast` in `kestrel/`). Everything below differs from the design above; anything not listed is as designed.

- **`par for` (A).** The design kept "moving an outer local in a `par for` body is always the loop error", which the old checker found at the move. A's loop rule finds it at the back edge instead, and a body that never finishes (`while true { }` with no `break`) has no back edge, so the final review found a move of an outer owning local that every thread made: a reader of the zeroed value failed at run time. As built, a move of a local declared outside the innermost `par for` body is refused at the move, whatever follows it: "`t` is declared outside this `par for`, whose iterations run at the same time, so they can't each move it", with help "declare it inside the loop, so each iteration has its own". A local declared inside the body (or inside an ordinary loop in it) follows the usual rules, and the loop error's help inside a `par for` body says "leave the loop with `break`" without suggesting `return`, which a body can't use.
- **A default loop reported twice (C).** A default that builds the same thing twice (`kids: [Node] = [Node(), Node()]`) reported its loop twice; the cycle check now follows each thing a default builds once, so every loop is reported once. Two fields that each close a loop through the same constant are two loops, each reported once.
- **`let A = Arena()` as a default (C).** C's "no owning defaults" check treated a constant of an owning type as building an owning value, so `let A = Arena()` with `mem: Arena = A`, which compiled before, was refused. A constant whose type is exactly `Arena` is a fresh `Arena()` at each use, like writing it out, so it is allowed again; a constant that holds an arena inside a struct is still refused (`tests/programs/arena_constant_default.kes`).
- **Construction messages (C, D).** A capitalised call that is neither a struct nor a function counts as a constant expression in a constant or field default, so `let X = Ball(1)` says "unknown struct `Ball`" there too instead of the "constants can only use..." message. An enum called like a struct says "`State` is an enum, so it can't be called like a struct", with help naming a real variant ("build a value from one of its variants, like `State.Idle`", or `Shape.Circle(...)` when the variant has fields); a constant called like a function says "`RED` is a constant, not a function", with help "use it without `()`: `RED`".
- **D3's help.** Built-in scalar types (`int`, `i64`, `u8`, `float`, `f64`, `bool`, `string`, and `GpuHandle`) get the message but no "write `let x: int = int()`" help, since they can't be built by calling them. A top-level `let ORIGIN: vec3()` gets the same message and help as a local `let` (it said "expected `=`, found `(`").
- **The views program test (B).** `tests/programs/let_else_views.kes` unwraps an optional `Pool` field that owns an `Arena`, not an optional texture field, so it is an owning payload that needs no window.
- **A depth limit (added in the fix round).** The user found that 50,000 nested parentheses or a 100,000-term `1 + 1 + ...` overflowed the compiler's stack. The parser now refuses code nested more than 1,000 levels deep, so no later pass recurses over a deeper tree: "this expression nests more than 1,000 levels deep; split it into smaller parts with `let`", and for types "this type nests more than 1,000 levels deep; simplify it". It counts the constructs open around each token (brackets, call arguments, blocks other than a function's body, unary operators, `if`, `match`, string interpolation) plus each expression's height, which also counts what the parser builds in loops: operator chains (`a + b + c` nests to the left) and `.field`, `.method()` and `[i]` chains. The error is reported once, at the line where the limit passed, and the rest of the file isn't parsed. 1,000 levels compile and run in the debug build (`tests/programs/depth_limit.kes`); `cli/tests/depth.rs` generates the deep inputs. Not chosen: a bigger stack, since such code only comes from generators.

- **Loops (A).** The plan's `loop_body(entry, body, exits_at_start)` used the state before a `while` condition as the "condition false" exit state. A condition that moved a value, with a body that always `return`s, then let later code use the moved value: the review found `while Pair(b: t).b.n > 100 { return }` followed by `print(t.n)` compiled and printed 0. As built it is `loop_body(entry, exit_at_start: Option<Moved>, body)`. `entry` (the state before the condition) is the back-edge baseline. The exit state is the state after the condition for `while`, none for `while true`, and the entry state for the `for` forms.
- **`let ... else`'s `else` block (A).** It must leave, so what it moves doesn't flow past it: the checker saves the move state before the block and restores it after. The design only said the block sees the state before the unwrap. (Added in Task 1's review round.)
- **Receivers (B).** Views rely on their root not changing, and one case slipped through an older gap: a method that takes `self` (by pointer) with the same variable also passed as `var` (`c.poke(var c)`) was not an exclusivity error, and a view of `self.b` then read freed memory. As built, a non-`var` receiver passed by pointer counts against `var` arguments, with the message "`c` is passed as `var` and is also the receiver of this call" and help "copy what the method needs into a separate variable first".
- **Moving an owning view at a tail (B).** The "it's inside `c`" message is keyed on `Tables::let_else_views` and the view's root, not on the live guards, so a block or function tail gets it too.
- **Enum values in constants (C).** The checker's constant ordering (`names_used`) and KIR's (`uses_in`) follow an enum value's arguments. Without that, `let LOADOUT = Kit.Bag(IDS)` declared before `IDS` was an internal compiler error; `tests/programs/enum_constant_order.kes` pins it.
- **File names (E).** The `FileName` record is emitted with alignment 8, because `cranelift-jit` places read-only data at alignment 1 and the runtime reads the record through an aligned pointer. `kir/tests/pack_line.rs` tests the packing of file and line.
- **The full check** passed at the end of the six tasks: 583 tests, clippy clean in `kestrel/` and `kestrel/godot`, the add-on rebuilt, and its headless test with 30 checks and 0 failures (part E changed the runtime's error functions, so the add-on had to be re-run). After the fix round: 602 tests, clippy clean in both, and the add-on's headless test with 31 checks (one new: a runtime error names "of bridge_test.kes") and 0 failures.
- **The ledger** (`.superpowers/sdd/2026-10-12-language-pass-1/progress.md`, git-ignored) is deleted when the pass finishes. The record is the commits, this section and the follow-ups file's "Found while building the first language pass".
