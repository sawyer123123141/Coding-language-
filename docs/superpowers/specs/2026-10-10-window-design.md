# Standalone window design (sub-project 4b, part 1: 2D)

Status: decided and built (2026-10-10). The user picked the syntax on 2026-10-10: a program-owned frame loop, three modules (`window`, `draw`, `input`), Godot-style float colours, `held`/`pressed`/`released` input, global math like GDScript, and the `native` keyword. Everything else is Claude's rulings, recorded here. Where the build differs from the first design, the text below says what was built (the calling convention, headless output, and the extra environment variables).

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
- `close()` only asks: the next `frame()` closes the window and returns `false`.
- After the window has closed, by `close()` or by the user: `frame()` keeps returning `false`, drawing and `screenshot` are errors, the queries (`delta`, `time`, `size`, `fps`) keep the values they had at the end, and `input` reads as nothing.
- The window closes when the program ends.
- `frame()` also flushes the program's `print` output. `print` is buffered until the program ends, so without this a program that runs frames for minutes would show nothing meanwhile.

### `draw`

Pixels, origin top-left, y down (like Godot 2D). The colour is always the last argument.

```
draw.clear(color: Color)
draw.rect(pos: vec2, size: vec2, color: Color)
draw.fill(r: Rect, color: Color)
draw.rect_outline(pos: vec2, size: vec2, width: float, color: Color)
draw.circle(center: vec2, radius: float, color: Color)
draw.circle_outline(center: vec2, radius: float, width: float, color: Color)
draw.line(a: vec2, b: vec2, width: float, color: Color)
draw.triangle(a: vec2, b: vec2, c: vec2, color: Color)
draw.text(text: string, pos: vec2, size: float, color: Color)
draw.text_width(text: string, size: float) -> float
```

- `fill` draws a filled `Rect` (`pos` and `size`, like Godot's `Rect2` and `draw_rect`). It is written in Kestrel in `draw.kes` and calls `rect(r.pos, r.size, color)`, so headless output and errors (such as no window open) say `draw.rect`.
- `line` takes `a, b`, because `from` and `to` are keywords.
- `text` uses raylib's built-in font; `pos` is the top-left corner.
- Circles are drawn with raylib's `DrawCircleSector` (it picks enough segments for the radius), outlines with `DrawRing`, so `circle_outline` and `rect_outline` borders sit inside the shape. `triangle` accepts either winding. A rectangle with a negative size draws nothing yet (a follow-up).
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
- An interpolated string passed straight to a `native` function's `string` parameter is built in scratch memory that is freed after the call. So `draw.text("fps {window.fps()}", ...)` in the frame loop doesn't grow memory every frame. (Measured: 100,000 frames of `draw.text("n {n}", ...)` grew the program arena by 0 bytes; before the scratch arena the same loop grew it by 655,360 bytes, 640 KiB.)
- The checker treats such a call's arguments as a region that is freed when the call returns, like an `arena { }` block. So storing something made in an argument into a longer-lived variable is an error: "this memory is freed when the call to `draw.text` returns, but `x` lives longer".
- A `let label = "..."` inside the frame loop does allocate every frame. Wrap the loop body in `arena { }` when it builds strings or lists with `let`. The particles demo needs no `arena { }`, because it builds its text only inside `draw.text` arguments.
- Array, list and map constants are built once before `main` and read in place (3D spec, "Language additions"), so a palette like `let PALETTE = [draw.RED, draw.GREEN, draw.BLUE]` with `PALETTE[n % 3]` in the frame loop allocates nothing.

## Built-in modules and `native`

- `window`, `draw` and `input` are Kestrel files embedded in the compiler (`kestrel/driver/std/`). `use draw` loads the embedded file. A `draw.kes` next to the program is an error: "`draw` is a built-in module, so draw.kes can't be used as a module; rename the file".
- Their diagnostics show a virtual path, `<builtin>/draw.kes`. `Compiled.builtins` lists the built-in modules a program uses (the CLI picks the library from it, and the Godot add-on refuses them).
- They are ordinary Kestrel: structs (`Color`), enums (`Key`), constants (`WHITE`), Kestrel functions (`input.axis`), and `native` functions.
- A constant may construct a struct (`pub let RED = Color(1, 0, 0)`), which `draw.kes` needs. A struct field's default may also be a struct or enum value (not an owning one) since the first language pass (2026-10-12), so `tint: Color = draw.WHITE` works. When this spec was written a default could construct only a `vec2` or `vec3`.
- Constants are checked in dependency order, whichever file they are in and whatever order the `use` lines and the declarations come in, so `let BACKGROUND = draw.BLACK` works in a program and a module's `pub let SKY = draw.BLUE` works too. A constant that uses itself, directly or through others, is an error that names the loop: "constants can't use each other in a loop: A -> B -> A".
- `native` (user decision) declares a function whose body is supplied by the runtime: `pub native circle(center: vec2, radius: float, color: Color)`. It has no block. `pure native` is allowed, for a native with no effects.
- Only built-in modules may declare `native` functions: "only Kestrel's built-in modules can declare native functions". A native body in a user file is a later step, toward a C FFI.
- A `native` function's parameters are limited to what the runtime can read: integers, floats, `bool`, `vec2`, `vec3`, `string`, enums without fields, and structs of those (a struct may hold a `string`). A native can't take a `var` parameter. It may return nothing, a number, a `bool`, `vec2` or `vec3`; a `string` or an enum result isn't allowed yet.
- A native can't be a method. Write `pub pure native f(...)`, with `pub` first.
- The native's name is its symbol, `kestrel_<module>_<name>` (`kestrel_draw_circle`). The runtime's Rust functions have that name and `extern "C"`.
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
  - Natives are imports with Kestrel's own calling convention, so codegen has no special case for them. This differs from the first design, which passed strings as a pointer and a length and enums as their tag:
    - scalars go in registers; `bool` and `u8` parameters are zero-extended;
    - a `vec2`, a `Color` or any struct argument is a pointer to the value;
    - a `string` argument is a pointer to its 16-byte header, `{ptr, len}`, and `ptr` is null when `len` is 0 (an empty interpolation, or a zero-filled `array[string]` element), so a native must never build a slice from a null pointer;
    - an enum argument is a pointer to its value, with the tag in the first byte (`Key` is a tag byte, so `input.held(key)` reads `*key`);
    - a `vec2` result is written through a pointer that is passed as the first argument, before the others;
    - a native must not write through a read-only aggregate argument, because it may be the caller's own variable.
  - The cost of this choice is one more indirection in a native for a string or an enum. If it matters, natives could take a pointer and a length later without changing the Kestrel side.
  - `sqrt floor ceil abs min max` use Cranelift instructions, and `round` is built from `trunc` (Cranelift's `nearest` rounds halves to even). `sin cos tan asin acos atan atan2 exp log pow` call the runtime.
  - Format expressions reuse the print path: the runtime keeps a separate per-thread stack of string buffers in `runtime/src/print.rs`. The print sink is not part of it and is unchanged. `kestrel_rt_str_begin` pushes a buffer, the existing print calls write into the top one while any is open (and into the sink otherwise), and `kestrel_rt_str_end` pops it and copies it into the current arena. Buffers nest, so an interpolated string inside another one works.
- The JIT gets the native table from the CLI, so the runtime's own list of symbols doesn't grow by one entry per native.

### Runtime behaviour

- One window per program, used from the thread that opened it. That is the program's thread: natives can't run in `par for` bodies.
- The window runs on the CLI's big-stack thread, which GLFW allows on Windows. (macOS needs the main thread; that's for the cross-platform step.)
- Runtime errors from natives name the function, not the line. Lines for natives are a follow-up.
- Natives write their output through `kestrel_runtime::print`, never `println!`, so it stays in order with the program's own `print`.
- raylib logs at error level only (`SetTraceLogLevel(LOG_ERROR)` before `InitWindow`), so it never writes to stdout.
- A built window program is about 812 KB in release (a plain program is about 174 KB), because it links raylib.
- A runtime error while the window is open prints as usual and exits with code 101; the window goes with the process.

## Testing

- **Headless mode:** `KESTREL_HEADLESS=<n>` makes `window.open` open nothing and `frame()` return `true` `n` times. `window.open`, `window.close`, `window.screenshot` and every `draw.*` call then print one line into the program's output, in order with `print`: the function's name, then its arguments as `print` would print them, except that **a string is printed in double quotes** (`draw.text "fps 60" ...`, so empty and spaced strings are visible). The queries (`delta`, `time`, `size`, `fps`) print nothing. Input reads as nothing pressed, mouse `(0, 0)`, wheel 0; `delta()` is 1/60, `time()` is frames/60, `fps()` 60, `size()` the opened size; `screenshot` prints its path. `draw.text_width` prints a line too, and returns the text's length times `size * 0.5`.
- **Environment variables.** The first three are read at `window.open`; the last two belong to the runtime, which reads `KESTREL_ARENA_REPORT` once `main` has returned and `KESTREL_THREADS` when the `par for` pool starts:
  - `KESTREL_HEADLESS=<n>`: headless, with `n` frames. A value that isn't a whole number is an error.
  - `KESTREL_HEADLESS_QUIET=1`: a headless window prints **none** of its lines (`open`, `close`, `screenshot` and every `draw.*` call). A program that draws thousands of times a frame can then run headless without flooding its output. The program's own `print` is not affected.
  - `KESTREL_HIDDEN=1`: a real window that isn't shown. It still renders, and screenshots work.
  - `KESTREL_ARENA_REPORT=1` (the runtime, not the window): prints `kestrel: program arena <bytes> bytes` to stderr after `main` returns, and measures the arena only then. The memory test uses it; set it only in tests.
  - `KESTREL_THREADS=<n>`: the `par for` thread count (older than the window).
- **Program tests** (`tests/programs`, JIT and exe) gain an `// env: KEY=VALUE` directive (one per line, applied to both runs) and cover the modules, math, random with `rand_seed`, and interpolated strings. The harness is hermetic: every CLI test that runs `kestrel` or a built program first removes all `KESTREL_*` variables from the child's environment (`cli/tests/common`), then applies the file's `// env:` lines. A program that calls `window.open` without a `// env: KESTREL_HEADLESS=...` line makes the harness panic with a message, instead of opening a real window.
- **Error tests:** `native` in a user file, a module file shadowing a built-in, a reserved name, drawing from a `pure` function and from a `par for` body, math type errors, a native type outside the limits.
- **Window crate unit tests:** colour conversion, key mapping, the native table.
- **Memory test** (`cli/tests/text_memory.rs`): 100,000 frames of `draw.text("n {n}", ...)` against 1,000, compared by the arena report. A control program that stores a string each frame must grow, which shows the measurement works.
- **GPU smoke test** (`cli/tests/window_smoke.rs`): a real window draws a red rectangle, takes a screenshot, and the test checks a pixel of the PNG, under the JIT and as an exe. It tries a hidden window first; a visible one is used only when the hidden run exited 0 with the expected output and just the pixels were wrong. A second program draws a filled circle, a line, a triangle in each winding, a circle outline and some text, and checks a pixel in or beside each (and that the text area is not all black). On the development machine (Windows 11, OpenGL 3.3) the hidden window works for both.
- **The Godot add-on** refuses these modules, and its headless test (`demo/tests/test_bridge.gd`) checks that (27 checks).

## Demo

`kestrel/demos/particles/particles.kes`: thousands of particles.
- A `par for` moves them with a `pure step(var self, dt, target)`.
- The mouse attracts them; Space adds more.
- They're drawn as small rectangles under a text overlay with the fps and the particle count.
- The README section says how to run it. `kestrel/demos/particles/results.md` records the times at a few particle counts, honestly, including where drawing (not Kestrel) is the bottleneck.
- The demo prints one line a second (`particles 50000 update 0.41 ms draw 8.68 ms frame 16.66 ms fps 60.03`) so it can be measured from a script, and the same numbers are in the overlay. The loop body has no `let` strings, so it doesn't need an `arena { }`.
- **Result** (Ryzen 7 5800X, RTX 3070, 60 Hz, `--release`): 10,000 and 50,000 particles run at 60 fps with the update at 0.2 and 0.4 ms and drawing at 1.9 and 8.7 ms; 100,000 is just inside the 60 fps frame (drawing 15.9 ms); 200,000 runs at 30-33 fps (drawing 30 ms, update 0.9 ms). The update is under 3% of the work, and about 90% of the draw time is inside raylib and the graphics driver rather than in the Kestrel loop or the native call. A bulk draw call or textured batches would be the way to go faster.

## Build order

Two plans:
1. **Language pieces** that need no window and also work in Godot: math, vector methods, random numbers, and interpolation anywhere.
2. **The window:** `native`, built-in modules, vendored raylib, the window crate, linking, headless mode, the smoke test, and the demo.

## Not in this spec

Textures and sprites, 3D (camera, meshes, the voxel-world viewer: next spec), audio, gamepads, fonts, resizing callbacks, a windowed (no console) exe, user-declared natives / C FFI, hot reload of a running standalone game, Linux and macOS builds.
