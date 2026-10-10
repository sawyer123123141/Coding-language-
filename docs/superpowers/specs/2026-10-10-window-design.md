# Standalone window design (sub-project 4b, part 1: 2D)

Status: decided. The user picked the syntax on 2026-10-10: a program-owned frame loop, three modules (`window`, `draw`, `input`), Godot-style float colours, `held`/`pressed`/`released` input, global math like GDScript, and the `native` keyword. Everything else is Claude's rulings, recorded here.

**Scope ruling:** this spec is 2D: a window, shapes, text, keyboard and mouse, global math, random numbers, and string interpolation anywhere. Textures, 3D meshes and a camera (the voxel-world viewer) are the next spec, so each stays plan-sized.

## What it looks like

```
use window
use draw
use input
use { Color, WHITE from draw }
use { Key from input }

main() {
    window.open(800, 600, "Kestrel")
    var pos = vec2(400, 300)
    var angle = 0.0
    while window.frame() {
        let dt = window.delta()
        pos.x += input.axis(Key.Left, Key.Right) * 200 * dt
        if input.pressed(Key.Space) { angle = 0 }
        angle += TAU * dt

        draw.clear(Color(0.08, 0.08, 0.12))
        draw.circle(pos + vec2(cos(angle), sin(angle)) * 40, 10, draw.ORANGE)
        draw.circle(pos, 20, Color(1, 0.3, 0.3))
        draw.text("fps {window.fps()}", vec2(10, 10), 20, WHITE)
    }
}
```

## The modules

### `window`

```
window.open(width: int, height: int, title: string)
window.frame() -> bool
window.close()
window.delta() -> float
window.time() -> float
window.size() -> vec2
window.fps() -> int
window.screenshot(path: string)
```

- `open` makes a resizable window with vsync on. Calling it twice is an error ("the window is already open").
- `frame()` finishes the previous frame (shows it and reads input), then starts the next one and returns `true`. It returns `false` once the window has been closed, by the user or by `window.close()`, and keeps returning `false` after that. The first call only starts the first frame.
- Escape doesn't quit (raylib's default does; Godot's doesn't).
- `delta()` is the seconds since the last frame (0 in the first). `time()` is the seconds since `open`, as a `float`: millisecond-accurate for hours, and it avoids `float(...)` conversions everywhere.
- `screenshot` saves what has been drawn so far this frame as a PNG, right away (so a program can draw, screenshot, and quit).
- Calling anything but `open` before `open`, or any draw call after the window has closed, is a runtime error naming the function: "draw.circle was called before window.open".
- The window closes when the program ends.

### `draw`

Pixels, origin top-left, y down (like Godot 2D). The colour is always the last argument.

```
draw.clear(color: Color)
draw.rect(pos: vec2, size: vec2, color: Color)
draw.rect_outline(pos: vec2, size: vec2, width: float, color: Color)
draw.circle(center: vec2, radius: float, color: Color)
draw.circle_outline(center: vec2, radius: float, width: float, color: Color)
draw.line(a: vec2, b: vec2, width: float, color: Color)
draw.triangle(a: vec2, b: vec2, c: vec2, color: Color)
draw.text(text: string, pos: vec2, size: float, color: Color)
draw.text_width(text: string, size: float) -> float
```

- `line` takes `a, b`, because `from` and `to` are keywords.
- `text` uses raylib's built-in font; `pos` is the top-left corner.
- `Color` is a struct in `draw`: `r g b: float`, `a: float = 1`, channels from 0 to 1 (values outside are clamped when drawing). Named colours are constants in `draw` with Godot's values: `WHITE BLACK GRAY RED GREEN BLUE YELLOW ORANGE PURPLE TRANSPARENT`.

### `input`

```
input.held(key: Key) -> bool
input.pressed(key: Key) -> bool
input.released(key: Key) -> bool
input.mouse() -> vec2
input.wheel() -> float
input.axis(negative: Key, positive: Key) -> float
```

- `pressed` and `released` mean "since the last frame" (Godot's `is_action_just_pressed`).
- `Key` is an enum in `input`: `A`–`Z`, `Digit0`–`Digit9` (names can't start with a digit), `Space Enter Escape Tab Backspace Shift Ctrl Alt Up Down Left Right`, `F1`–`F12`, and the mouse buttons `MouseLeft MouseRight MouseMiddle`. `Shift`, `Ctrl` and `Alt` match either side.
- `mouse()` is in window pixels. `wheel()` is this frame's scroll, positive away from the user.
- `axis` is -1, 0 or 1, like Godot's `Input.get_axis`; it's written in Kestrel inside `input.kes`.

## Global math and random numbers

No import needed, like GDScript. They are built into the compiler, like `int()`, so they can accept several number types, which user functions can't.

| Function | Takes | Gives |
|---|---|---|
| `sqrt sin cos tan asin acos atan exp log floor ceil round` | `float` or `f64` (an integer becomes `float`) | the same type |
| `atan2(y, x)`, `pow(x, y)` | two `float`s or two `f64`s | the same type |
| `abs sign` | any number | the same type |
| `min max` | two numbers (widened like `+`) | their type |
| `clamp(x, lo, hi)` | three numbers (widened like `+`) | their type |
| `lerp(a, b, t)` | `a, b`: `float`, `f64`, `vec2` or `vec3`; `t`: `float` (or `f64` with `f64`) | the type of `a` |
| `PI`, `TAU` | | `float` constants |

- All of these are `pure`, so `pure` functions and `par for` bodies may use them.
- An integer argument where a float is needed becomes a `float`, as elsewhere.
- `abs` of the smallest integer overflows: an error with checks on, like `-x`.
- `round` rounds halves away from zero, like GDScript.
- `clamp(x, lo, hi)` is `min(max(x, lo), hi)`, so `hi` wins when `lo > hi`.
- `min`, `max` and `clamp` check each argument with the running common type as a hint, like the right side of `+`, so `min(small_u8, 7)` is a `u8`. The hint only flows left to right, and the call doesn't get the type its result is wanted as. So `min(7, small)` is an `int`, and `let b: u8 = min(7, small)` is an error while `let b: u8 = min(small, 7)` works.
- `lerp`'s `t` is coerced to the result type, so `lerp(a_float, b_float, t_f64)` is an error.
- `PI` and `TAU` are exact when they meet an `f64`. A named float constant is a `float`, so `let HALF = PI / 2` widens from `float` when it meets an `f64`.
- Vectors get GDScript's methods: `v.length()`, `v.normalized()` (a zero vector stays zero, as in Godot), `a.dot(b)`, `a.distance_to(b)`, on `vec2` and `vec3`.
- Random numbers, global like GDScript: `randf()` (0 up to but not including 1, so `int(randf() * n)` never indexes past the end), `randf_range(lo, hi)`, `randi_range(lo, hi)` (both ends included, as in Godot). The generator is seeded from the clock at startup, like Godot 4; `rand_seed(n)` makes the sequence repeatable. The generator is per thread. They are not `pure`, because each call changes the generator.
- These names are reserved, like `print`, through the same check: a function, type, constant, variable or parameter with one of them is an error ("`min` is a built-in name; pick another", the message `print` and `Arena` already use). The vector method names (`length`, `normalized`, `dot`, `distance_to`) are not reserved, so a field can still be called `length`. `seed` is not reserved either, because programs already use it as a parameter name.

## String interpolation anywhere

- `"hp {hp}"` is now a `string` value anywhere, not only in `print`: `let label = "hp {hp}"`. It formats exactly as `print` does and is built in the current arena, like a list.
- Strings are heap values now, like arrays. Copying a string place copies its bytes into the current arena, literals stay in read-only data and belong to no arena, and the arena escape check covers strings. So storing a non-literal string into a longer-lived place needs `in x.mem { }`, for example `self.name = n` through `var self`. A map copies a string key when the key adds an entry.
- An interpolated string passed straight to a `native` function's `string` parameter is built in scratch memory that is freed after the call. So `draw.text("fps {window.fps()}", ...)` in the frame loop doesn't grow memory every frame.
- A `let label = "..."` inside the frame loop does allocate every frame. Wrap the loop body in `arena { }` when it builds strings or lists; the demo shows this.

## Built-in modules and `native`

- `window`, `draw` and `input` are Kestrel files embedded in the compiler (`kestrel/driver/std/`). `use draw` loads the embedded file. A `draw.kes` next to the program is an error: "`draw` is a built-in module, so draw.kes can't be used as a module; rename the file".
- They are ordinary Kestrel: structs (`Color`), enums (`Key`), constants (`WHITE`), Kestrel functions (`input.axis`), and `native` functions.
- `native` (user decision) declares a function whose body is supplied by the runtime: `pub native circle(center: vec2, radius: float, color: Color)`. It has no block. `pure native` is allowed, for a native with no effects.
- Only built-in modules may declare `native` functions: "only Kestrel's built-in modules can declare native functions". A native body in a user file is a later step, toward a C FFI.
- A `native` function's types are limited to what the runtime can read: integers, floats, `bool`, `vec2`, `vec3`, `string`, enums without fields, and structs of those. It may return a scalar, `vec2`, `vec3` or an enum.
- A native that isn't `pure` has effects (like `print`), so `pure` functions and `par for` bodies can't call it. Drawing from a `par for` body is therefore a compile error.

## How it's built (Claude's rulings)

### raylib

- **Vendored source, built with the `cc` crate.** raylib 6.0's `src/` (zlib license, taken from the `raylib-sys` 6.0.0 crate) lives in `kestrel/window/raylib/`, minus audio and the unused platform backends. `kestrel/window/build.rs` compiles it for desktop OpenGL 3.3.
- **No bindgen.** `raylib-sys` runs bindgen, which needs libclang (LLVM), and this machine has none. A hand-written `extern "C"` block declares the few dozen raylib functions used. Cost if wrong: about 10 MB of C in the repo; switching to `raylib-sys` later touches one crate.
- Windows system libraries: `opengl32 gdi32 winmm user32 shell32`.

### Crates

- **`kestrel-window`** (`kestrel/window/`, rlib and staticlib) depends on `kestrel-runtime` and holds the raylib build and the native functions, named `kestrel_<module>_<name>` (`kestrel_draw_circle`).
  - It exposes a table of native symbols for the JIT.
  - Its staticlib contains the runtime too, so a program that uses a window links `kestrel_window.lib` *instead of* `kestrel_runtime.lib` (two Rust staticlibs would each bring their own copy of std).
- **CLI:** `kestrel run` links `kestrel-window` and registers its symbols with the JIT. `kestrel build` links `kestrel_window.lib` plus the system libraries when the program uses `window`, `draw` or `input`, and the runtime library otherwise. `cli/build.rs` builds both staticlibs.
- **Godot add-on:** doesn't link `kestrel-window`. Loading a file that uses `window`, `draw` or `input` is a load error: "window, draw and input can't be used inside Godot". Math, random numbers and interpolation work there.

### Compiler

- **Syntax:** `native` is a keyword. A function declaration's body becomes optional, and only `native` functions omit it.
- **Driver:** embeds the three module sources and resolves `use` to them. It reports which built-in modules a program uses, so the CLI picks the library and the add-on can refuse.
- **Check:**
  - native declarations and their type limits;
  - math builtins with their typing;
  - reserved names;
  - interpolated strings as `string` expressions.
- **KIR:** `Func` gets a `native` flag; a call to it lowers as a call to the import `kestrel_<module>_<name>`. Math lowers to new expression nodes. An interpolated string lowers to a format expression; when it's a direct argument to a native, the call is wrapped in a scratch-arena push and pop.
- **Codegen:**
  - Natives are imports with Kestrel's own convention: scalars in registers, aggregates (`vec2`, structs) by pointer, strings as pointer and length, enums as their tag.
  - `sqrt floor ceil abs min max` use Cranelift instructions, and `round` is built from `trunc` (Cranelift's `nearest` rounds halves to even). `sin cos tan asin acos atan atan2 exp log pow` call the runtime.
  - Format expressions reuse the print path: the runtime keeps a separate per-thread stack of string buffers in `runtime/src/print.rs`. The print sink is not part of it and is unchanged. `kestrel_rt_str_begin` pushes a buffer, the existing print calls write into the top one while any is open (and into the sink otherwise), and `kestrel_rt_str_end` pops it and copies it into the current arena. Buffers nest, so an interpolated string inside another one works.
- The JIT gets the native table from the CLI, so the runtime's own list of symbols doesn't grow by one entry per native.

### Runtime behaviour

- One window per program, used from the thread that opened it. That is the program's thread: natives can't run in `par for` bodies.
- The window runs on the CLI's big-stack thread, which GLFW allows on Windows. (macOS needs the main thread; that's for the cross-platform step.)
- Runtime errors from natives name the function, not the line. Lines for natives are a follow-up.
- A runtime error while the window is open prints as usual and exits with code 101; the window goes with the process.

## Testing

- **Headless mode:** `KESTREL_HEADLESS=<n>` makes `window.open` open nothing and `frame()` return `true` `n` times. Each draw call then prints one line into the program's output, in order with `print`: the function's name, then its arguments as `print` would print them. Input reads as nothing pressed, mouse `(0, 0)`, wheel 0; `delta()` is 1/60, `fps()` 60, `size()` the opened size; `screenshot` prints its path.
- **Program tests** (`tests/programs`, JIT and exe) gain an `// env: KESTREL_HEADLESS=3` directive and cover the modules, math, random with `rand_seed`, and interpolated strings.
- **Error tests:** `native` in a user file, a module file shadowing a built-in, a reserved name, drawing from a `pure` function and from a `par for` body, math type errors, a native type outside the limits.
- **Window crate unit tests:** colour conversion, key mapping, the native table.
- **GPU smoke test** (CLI test): a real window draws a red rectangle, takes a screenshot, and the test checks a pixel of the PNG. It tries a hidden window first; if a hidden window reads back blank, it uses a small visible one.

## Demo

`kestrel/demos/particles/particles.kes`: thousands of particles.
- A `par for` moves them with a `pure step(var self, dt, target)`.
- The mouse attracts them; Space adds more.
- They're drawn as small rectangles under a text overlay with the fps and the particle count.
- The README section says how to run it. The plan records frame times at a few particle counts, honestly, including where drawing (not Kestrel) is the bottleneck.

## Build order

Two plans:
1. **Language pieces** that need no window and also work in Godot: math, vector methods, random numbers, and interpolation anywhere.
2. **The window:** `native`, built-in modules, vendored raylib, the window crate, linking, headless mode, the smoke test, and the demo.

## Not in this spec

Textures and sprites, 3D (camera, meshes, the voxel-world viewer: next spec), audio, gamepads, fonts, resizing callbacks, a windowed (no console) exe, user-declared natives / C FFI, hot reload of a running standalone game, Linux and macOS builds.
