# `par for` design (sub-project 3)

Status: decided. The syntax and body rules come from the syntax spec (`par for`). On 2026-10-09 the user also decided that `pure` functions may take `var` parameters. Everything else is Claude's rulings, recorded here.

## What it looks like

```
struct Chunk {
    x: int
    z: int
    mem: Arena = Arena()
    faces: int = 0

    pure build(var self, seed: int) {
        arena {
            self.faces = mesh(self.x, self.z, seed).indices.len / 6
        }
    }
}

main() {
    var chunks = list[Chunk]()
    for x from 0 to 8 { for z from 0 to 8 { chunks.add(Chunk(x: x, z: z)) } }
    par for c in chunks {
        c.build(seed: 1337)
    }
    var faces = 0
    for c in chunks { faces += c.faces }
    print(faces)
}
```

## Rules

### `pure` with `var` parameters (user decision)

- A `pure` function may take `var` parameters, including `var self`. `pure` now means: no printing, and nothing changes except the function's own locals and what it's handed as `var`.
- A call to a `pure` function whose result is unused is an error only when the function has no `var` parameters, since otherwise the call changes something.
- This makes `mesh` and `quad` in `bench/voxels3d` `pure`.

### The loop

- `par for v in xs` and `par for i, v in xs`, where `xs` is an array or a list.
  - Maps and `from ... to` ranges are errors.
  - Map: "`par for` loops over an array or a list".
  - Range: "`par for` needs items to work on; loop over an array or list".
- Iterations run at the same time, on all cores, in no particular order. The loop finishes when every iteration has.
- `KESTREL_THREADS` sets the number of threads (default: the machine's logical cores); `KESTREL_THREADS=1` runs the loop in order on the calling thread.
- A `par for` that runs inside another one (directly, or in a function it calls) runs its iterations in order on the thread it's on.

### The body

The body may:

- **change** the loop's item (`v`, through fields, elements and `var` arguments), and its own variables;
- **read** anything, except other items of the collection while the body changes items;
- **call** `pure` functions only, with `var` arguments rooted at the item or the body's own variables;
- use `arena { }`, and `in x { }` when `x` is rooted at the item or the body's own variables;
- use `continue`, which ends this iteration.

Each of these is a compile error at the line that does it:

| What | Message |
|---|---|
| Changing anything else | "`par for` iterations run at the same time, so this can't change `world.score`; only the loop's item (`c`) and the body's own variables can change" |
| Calling a function that isn't `pure` | "`par for` bodies can only call `pure` functions; `tick` isn't `pure`" |
| `print` | "`par for` bodies can't print: iterations run at the same time" |
| `return` | "`return` can't leave a `par for` body" |
| `break` | "`break` can't stop a `par for`: iterations run at the same time" |
| Reading the collection by name while the body changes items | "this reads `chunks`, whose items other iterations are changing; use the loop's item (`c`)" |

- **Inner loops:** a loop inside the body over something rooted at the item (`for h in c.heights`) gives views that count as the item. Views into anything else are read-only.
- **Panics:** an iteration that panics (overflow, a bad index) stops the loop. The program then fails with that iteration's message, as if the loop had run in order. When several iterations fail, which message is reported is unspecified.

### Memory

- Anything the body allocates in the current arena lives as long as the arena that was current when the `par for` started, exactly as in a plain `for`, so the escape check is unchanged.
  - Each thread allocates into a temporary arena while the loop runs.
  - When the loop finishes, those arenas' blocks move into that arena.
- `arena { }` and `in c.mem { }` inside the body work as usual.
- Collections grow in their own arenas, as always. Growing one whose arena is shared (the program arena, or an arena from outside the loop) takes a lock. Growth is geometric, so this is rare.

## How it works

### Syntax

`Stmt::ForIn` gains `par: bool`. `par` must be followed by `for` and a for-in form. A range after `par for` is an error.

### Check

- `BodyChecker` gets a stack of `ParBody` entries, one per enclosing `par for`. Each records:
  - the item's local;
  - the collection's root local;
  - the first local id declared in the body;
  - the inner views rooted at the item;
  - whether the body changes the item;
  - where the body reads the collection by name.
- **Ownership:** a place is the body's own when its unaliased root is the item, an inner view rooted at the item, or a non-view local declared in the body.
  - Every write checks this: assignment, map set, `add`/`remove` receivers, `var self` receivers, `var` arguments and `in`.
- The purity checks (`check_callee_purity`, `check_print`) also apply inside a `par for` body.
- `loop_breaks` gains each loop's kind, so `break` can tell when the innermost loop is a `par for`. `return` checks whether any `par for` is open.
- `collect.rs` stops rejecting `var` parameters on `pure` functions. `check_discarded_call` looks at the callee's parameters.

### KIR

- The body is outlined into a new function `__kestrel_par_N`:
  1. The loop is first lowered in place, as a `ForRange` from a `start` local to an `end` local, with the usual element binding.
  2. Locals created while lowering it move to the new function.
  3. Older locals it uses become its leading `RefParam` parameters (captures), followed by `start` and `end` (i64).
  4. Captured scalar locals in the caller become `Addressed`, so they have an address.
- The caller gets `Stmt::ParFor { func, args, count }`:
  - `args` are `Arg::Ref` places, one per capture;
  - `count` is the collection's length (i64);
  - `Module::par_bodies` lists the outlined functions.
- `visit`, `rename` and `inline` walk `ParFor` arguments. The outlined function is never a `Call` target, so it's never inlined.

### Codegen

- For each function in `par_bodies`, a thunk: `extern "C" fn(env: *const *mut u8, start: i64, end: i64)`. It loads each capture's address from `env` and calls the function.
- `ParFor` builds `env` on the stack (the captures' addresses) and calls `kestrel_rt_par_for(count, thunk, env, line)`.

### Runtime (`runtime/src/par.rs`)

- **Pool:** threads start lazily, with large stacks like the main thread's, and live for the process. Thread count is `KESTREL_THREADS`, else `available_parallelism()`.
- **`kestrel_rt_par_for(count, body, env, line)`:**
  1. Runs inline when `count` is 0 or 1, when there is one thread, or when this thread is already inside a `par for` job.
  2. Otherwise builds a job (`Arc`): an atomic next-index, a chunk size of `max(1, count / (threads * 8))`, a failure slot, the caller's program arena, a lock for shared growth, and the finished threads' arenas.
  3. Hands the job to `threads - 1` pool threads and works on it itself.
  4. Each participant runs with the job in a thread local: it pushes a fresh arena, takes chunks until none are left or the job failed, pops the arena and hands it to the job.
  5. The caller waits for every participant, then:
     - when an iteration failed, calls `fail` with that message on its own thread (default: flush, exit 101; Godot: its hook);
     - otherwise moves every participant's arena blocks into its current arena (`Arena::absorb`).
- **A failure on a pool thread** records the message (the first one wins), marks the thread done, and parks it forever, since unwinding through compiled code isn't safe. The pool starts a replacement when it next runs short.
- **The caller failing** in its own chunk marks the job failed (so the others stop taking chunks), then fails as usual.
- **Inside a job:**
  - `program_arena()` returns the caller's program arena, not the thread's own.
  - List growth and map growth take the job's lock when the arena they grow in isn't the top of this thread's arena stack.

## Testing

- **Programs:**
  - `tests/programs/par_for.kes`: items changed in place, `pure` `var self` methods, allocation in the body that outlives the loop, `arena { }` and `in c.mem { }` in the body, a nested `par for`, `continue`, a list grown in the program arena from many iterations.
  - The JIT and the exe both run each program, with `KESTREL_THREADS` unset, then `=1`, with the same output.
- **Panic:** `tests/programs/par_for_panic.kes`, an index error in one iteration (exit 101, the message names the line).
- **Errors:** `tests/errors/par_for.kes`, one per rule in the table, plus the allowed cases checked clean in `check/tests`.
- **`pure`:** `pure` with `var` parameters, including the discarded-call rule.
- **Runtime units:**
  - every index runs exactly once, for many counts and thread counts;
  - arenas are merged;
  - failure messages come back;
  - nested jobs run inline.
- **Benchmark:** `bench/voxels3d`, with `par for` over the chunks. Kestrel 1 thread vs N threads, vs C# single-threaded and C# `Parallel.For`, recorded honestly.
