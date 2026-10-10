# Sub-project 4a: Kestrel inside Godot

**Goal:** a Godot 4 add-on that loads `.kes` files, compiles them in memory (JIT), and lets GDScript call their `pub` functions. Edits hot-reload while the game runs, and a Kestrel panic reports an error instead of crashing the editor. A demo project builds a voxel heightmap mesh from Kestrel-generated heights.

**Decided by the user (2026-10-10):** games run first through a Godot add-on, before `par for` and before a standalone window.

**Environment:** Godot 4.7.2 (Mono build) is on this machine. The add-on uses godot-rust (`godot` 0.5.5, feature `api-4-7`) and works with the standard (non-Mono) Godot too.

## Design

- **GDScript API** (a `RefCounted` class registered by the extension):
  ```gdscript
  var k := Kestrel.new()
  if k.load("res://voxels.kes"):                    # false, and errors printed, on failure
      var h: PackedInt32Array = k.run("heights", [cx, cz, 1337])
  k.functions()                                      # PackedStringArray of runnable names
  ```
  `run` recompiles first if the file or a module it uses changed on disk (hot reload). If the new version has errors, they're printed and the old version keeps running.
- **What GDScript can call:** `pub` functions of the loaded file whose parameter and return types bridge to Godot:

  | Kestrel | Godot |
  |---|---|
  | `int`, `i64`, `u8` | `int` |
  | `float`, `f64` | `float` |
  | `bool` | `bool` |
  | `string` | `String` |
  | `vec2`, `vec3` | `Vector2`, `Vector3` |
  | `[int]` / `list[int]` | `PackedInt32Array` |
  | `[float]` | `PackedFloat32Array` |
  | `[u8]` | `PackedByteArray` |
  | `[vec2]` | `PackedVector2Array` |
  | `[vec3]` | `PackedVector3Array` |

  Other `pub` functions are left out of `functions()`, and `run` on them reports why.
- **Export thunks (codegen):** each runnable function gets a generated wrapper `fn(args: *const u8, ret: *mut u8)`.
  - Arguments sit in a byte buffer at their natural offsets: scalars are loaded from it, and aggregates are passed by pointer into it.
  - The result is stored at `ret`, or `ret` is the result slot for aggregates.
  - The host only ever calls one signature.
  - Input arrays point straight at Godot's packed-array memory for the duration of the call (read-only parameters), so they aren't copied.
- **Execution:** each `Kestrel` instance has a worker thread with a big stack. Per call, the worker:
  1. pushes a fresh arena
  2. calls the thunk
  3. copies any array result's elements out
  4. frees the arena, so nothing leaks between calls
  5. returns the bytes and the captured `print` output, which is printed with Godot's `print`
- **Panics:** the runtime's failure path calls a host hook. The add-on's hook sends the message back and parks the worker forever. The call reports the error with `push_error` and returns `null`, and the next call starts a fresh worker. Ruling: parking the thread rather than unwinding — unwinding through JIT frames isn't safe — cost if wrong: one leaked parked thread per runtime panic.
- **Driver library:** module loading, linking, checking and lowering move out of the CLI into a `kestrel-driver` crate. Both the CLI and the add-on use it. It returns rendered diagnostics as text and the list of files with their modification times (for hot reload).
- **Demo:** `kestrel/godot/demo/` is a Godot project with the add-on installed. `voxels.kes` computes a heightmap per chunk, and `main.gd` turns it into one `ArrayMesh` per chunk. A headless test script (`godot --headless --script`) checks that load and run work and that output matches.

## Steps

1. **Runtime:** a failure hook (`set_failure_hook`), output capture (`take_output`), and every failure path routed through it.
2. **Driver crate:** move `cli/src/driver.rs`; the CLI uses it.
3. **KIR/codegen:**
   - `Func::exported` (`pub` and in the entry file).
   - Export thunks.
   - `JitProgram::exports()` with name, parameter types, return type, thunk pointer, and argument/result layout.
   - Unit tests calling thunks from Rust.
4. **Add-on crate** `kestrel/godot` (cdylib): the `Kestrel` class, the worker, Variant conversions, hot reload.
5. **Demo project and headless test;** a README section on installing the add-on.
