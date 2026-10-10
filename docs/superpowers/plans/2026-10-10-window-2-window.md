# Window plan 2: native functions, raylib, and the window

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Kestrel programs open a window and draw, with `use window`, `use draw` and `use input`, under both `kestrel run` and `kestrel build`.

**Architecture:**
- **Built-in modules.** `window`, `draw` and `input` are Kestrel files embedded in the driver. Their bodyless `native` functions are Rust `extern "C"` functions in a new crate, `kestrel-window`, which calls raylib.
- **raylib** is vendored C source, compiled with the `cc` crate. There is no bindgen: a hand-written `extern "C"` block declares the raylib functions used.
- **Calling convention.** A native has the same calling convention as any Kestrel function: scalars in registers and aggregates by pointer. A string is a pointer to its 16-byte header. An enum is a pointer to its value, with the tag in the first byte. An aggregate return goes through a pointer passed first. Symbols are `kestrel_<module>_<name>`.
- **JIT and linking.** The JIT gets the window crate's symbol table from the CLI. `kestrel build` links `kestrel_window.lib`, which contains the runtime as well, in place of `kestrel_runtime.lib`.
- **Headless mode** (`KESTREL_HEADLESS=<frames>`) prints each draw call instead of drawing, so program tests can check them.

**Tech Stack:** Rust workspace `kestrel/`, Cranelift 0.116, raylib 6.0 (C, zlib licence), `cc` 1.x (already in the cargo cache), MSVC.

**Spec:** `docs/superpowers/specs/2026-10-10-window-design.md`. Read it first. Plan 1 (`2026-10-10-window-1-language.md`) is done: math, random numbers, and strings anywhere.

## Global Constraints

- Source files stay under about 400 lines. Follow existing patterns, and split by responsibility.
- Error messages are plain sentences that say what to do.
- From `kestrel/`, run `cargo test --workspace` and `cargo clippy --workspace --all-targets -- -D warnings`. Also run `cargo clippy --all-targets -- -D warnings` in `kestrel/godot`. It is a separate workspace with its own lock file, and it uses `driver`, `codegen` and `runtime` but not `kestrel-window`.
- Program tests are `tests/programs/*.kes` with `// expect:` lines and run under both the JIT and an exe. Error tests are `tests/errors/*.kes` with `// error <line>: <message>`; put the expectations at the end of the file.
- Write files containing `\n` or `\\` with the Write/Edit tools, not shell heredocs.
- Never call a bare `link.exe`. Git's `link.exe` shadows the MSVC linker; `find-msvc-tools` locates the real one.
- **Strings into natives can be empty with a null pointer** (`ptr = null, len = 0`), for example an empty interpolation or a zero-filled `array[string]` element. Natives must never call `slice::from_raw_parts` on a null pointer.
- **A native must not write through a read-only aggregate argument.** It may be the caller's own variable.
- **Runtime output from a native** goes through `kestrel_runtime::print` (`kestrel_rt_print_str`, `kestrel_rt_print_f32`, ...), never `println!`. That keeps it in order with `print`.
- **raylib must not log to stdout:** set `SetTraceLogLevel(LOG_ERROR)` before `InitWindow`.
- **The Godot add-on** must keep compiling with `JitProgram::compile` and `Compiled` changes (update `kestrel/godot/src` when they change).
- The flaky `relocated` CLI test sometimes fails with "file in use" (OneDrive). Re-run it.

---

### Task 1: Vendor raylib and create the `kestrel-window` crate

**Files:**
- Create:
  - `kestrel/window/Cargo.toml`
  - `kestrel/window/build.rs`
  - `kestrel/window/src/lib.rs`
  - `kestrel/window/src/raylib.rs`, the FFI declarations
  - `kestrel/window/raylib/`, the vendored source
  - `kestrel/window/raylib/README.md`, saying where it came from
- Modify: `kestrel/Cargo.toml` (add the member `window`, and `cc = "1"` to `[workspace.dependencies]`)

**Vendoring:**
- Copy `src/` and `LICENSE` from `~/.cargo/registry/src/*/raylib-sys-6.0.0/raylib/` into `kestrel/window/raylib/`. This is raylib 6.0; `raylib.h` says `RAYLIB_VERSION "6.0"`.
- Then delete:
  - `raudio.c`
  - from `external/`: `miniaudio.h`, `dr_flac.h`, `dr_mp3.h`, `dr_wav.h`, `stb_vorbis.c`, `jar_mod.h`, `jar_xm.h`, `qoa.h`, `qoaplay.c`, `RGFW/` and `glad_gles2.h`
  - every file in `platforms/` except `rcore_desktop_glfw.c`
  - `Makefile`, `CMakeLists.txt`, `*.html`, `*.rc`, `*.rc.data`, `*.ico` and `build.zig*`
- If the build later needs a deleted file, restore just that file.
- `raylib/README.md` says: "raylib 6.0 source (zlib licence, see LICENSE), copied from the raylib-sys 6.0.0 crate; audio and unused platform backends removed."

**Build** (`build.rs`, using `cc::Build`):
- Files: `rcore.c rshapes.c rtextures.c rtext.c rglfw.c` from `raylib/`.
- Defines: `PLATFORM_DESKTOP_GLFW`, `GRAPHICS_API_OPENGL_33`, `SUPPORT_MODULE_RAUDIO=0` and `SUPPORT_MODULE_RMODELS=0`.
- Include paths: `raylib/` and `raylib/external/glfw/include`.
- Silence C warnings with `.warnings(false)`.
- Link the system libraries with `cargo:rustc-link-lib=dylib=` lines: `opengl32 gdi32 winmm user32 shell32` on Windows. Other targets need no lines yet; the add-on and the window are Windows-only for now.
- `cargo:rerun-if-changed=raylib`.

**Crate:**
- `kestrel-window`: `crate-type = ["rlib", "staticlib"]`, depending on `kestrel-runtime`, with build-dependency `cc`.
- `src/raylib.rs` declares the raylib types and functions in an `extern "C"` block. This task needs only `GenImageColor`, `ImageDrawPixel` (or `ImageDrawRectangle`), `ExportImage`, `LoadImage`, `GetImageColor` and `UnloadImage`; later tasks add the rest.
  - `#[repr(C)]` types: `Color { r, g, b, a: u8 }`, `Vector2 { x, y: f32 }`, and `Image { data: *mut c_void, width, height, mipmaps, format: i32 }`.
  - The `Font` struct may stay opaque behind the functions that take it by value. If so, declare it with its real C layout from `raylib.h`, because it is passed by value.

**Interfaces (Produces):**
- `pub fn png_pixel(path: &std::path::Path, x: i32, y: i32) -> Option<[u8; 4]>` in `src/lib.rs`. It loads a PNG with raylib's `LoadImage` (no window needed) and returns the RGBA at `(x, y)`, or `None` if the file can't be read. Task 7's smoke test uses it.

- [ ] **Step 1: Failing test.** A unit test, `png_round_trip`, in `src/lib.rs`:
  1. Make a 4×4 image with `GenImageColor` (opaque black).
  2. Draw one red pixel at (2, 1).
  3. `ExportImage` it to a temp path.
  4. Check that `png_pixel(path, 2, 1) == Some([255, 0, 0, 255])` and `png_pixel(path, 0, 0) == Some([0, 0, 0, 255])`.
  5. Check that a missing file gives `None`.
- [ ] **Step 2: Implement the vendoring, the build and the FFI.** Run `cargo test -p kestrel-window`. The first build compiles raylib, which takes a minute.
- [ ] **Step 3: Check that the staticlib carries the runtime's symbols.**
  1. Run `cargo build -p kestrel-window --lib`.
  2. Run `dumpbin /symbols` (find it next to the linker with `find-msvc-tools`), or `llvm-nm` if available, on `target/debug/kestrel_window.lib`.
  3. Confirm that `kestrel_rt_print_i64` and `InitWindow` are both defined.
  4. Record the command and result in the report. If either is missing, stop and report BLOCKED: the linking plan depends on it.
- [ ] **Step 4: Run** the workspace tests and both clippy runs. Commit "Vendor raylib 6.0 and add the kestrel-window crate".

### Task 2: `native` functions in the compiler

**Files:**
- Modify:
  - `syntax/src/token.rs` (the `Native` keyword: the `Tok` variant, `keyword()` and `symbol()`)
  - `syntax/src/ast.rs` (`FnDecl.is_native: bool`)
  - `syntax/src/parser/item.rs`: `pub? pure? native name(params) (-> T)?` at top level only, with no block. The body is an empty `Block` spanned at the end of the signature. A `native` method is the parse error "native functions can't be methods".
  - `check/src/tables.rs` (`FnInfo.is_native`)
  - `check/src/collect.rs` (set it; check the native's types)
  - `check/src/body/mod.rs` (`check_fn`: for a native, declare the parameters, then return before checking the body)
  - `kir/src/ir.rs` (`Func.native: Option<String>`, the import symbol; set it to `None` at the other seven `Func` construction sites in `lower/helpers.rs`, `lower/map_helpers.rs` and `lower/par.rs`)
  - `kir/src/lower/mod.rs` (set `native`)
  - `kir/src/inline.rs` (`inlinable` returns `None` for a native)
  - `codegen/src/lib.rs` (declare natives as `Linkage::Import` under their symbol, and skip them in the define loop)
  - `codegen/src/func/mod.rs` or a new `codegen/src/native.rs` (the signature, with I8 params `uext` as in `RtFuncs::declare`)
  - `codegen/src/jit.rs` (`JitProgram::compile_with`)
- Test:
  - `syntax/tests/parser.rs`
  - `check/tests/natives.rs` (new)
  - `codegen/tests/natives.rs` (new; it calls Rust natives through the JIT)

**Interfaces (Produces):**
- Symbol naming: `kestrel_` plus the function's qualified name with `.` replaced by `_`. `draw.circle` becomes `kestrel_draw_circle`. A native in the entry file (tests only) named `twice` becomes `kestrel_twice`.
- `pub fn native_symbol(qualified: &str) -> String` in `kir`, used by lowering and tests.
- `impl JitProgram { pub fn compile_with(m: &kir::Module, opts: Options, extra: &[(&str, *const u8)]) -> Result<JitProgram, String> }`. `compile` delegates to it with `&[]`. Each `extra` entry is registered with `JITBuilder::symbol`.
- `kir::Func.native: Option<String>`, which is `Some(symbol)` for a native.

**Rules:**
- **The type check of a native**, in `collect.rs`, when `is_native`. Each parameter's type is one of:
  - `int i64 u8 float f64 bool string`
  - a struct whose fields are all such scalars or `vec2`/`vec3` (`vec2` and `vec3` themselves count)
  - an enum with no variant fields.
  
  The return is nothing, a scalar, `vec2` or `vec3`. Anything else is the error "native functions can only take numbers, bools, strings, vectors, and enums or structs of those". A `var` parameter on a native is the error "native functions can't take `var` parameters".
- **Effects:** a native that isn't `pure` sets `io = true`. The existing purity rules then keep it out of `pure` functions and `par for` bodies.
- **Where natives are allowed** isn't decided here: the driver (Task 3) rejects natives outside built-in modules. The checker accepts natives anywhere, so tests can declare them in the entry file.

- [ ] **Step 1: Failing tests.**
  - **Parser:**
    - `pub native f(x: int) -> int` parses with `is_native` and an empty body.
    - `pure native g()` parses.
    - `native` inside a struct body gives "native functions can't be methods".
    - `native h() { }` (with a body) gives "native functions have no body; remove the `{ }`".
  - **Check:**
    - A native taking `list[int]` and one returning `string` each give the type error.
    - A native taking `var x: int` gives "native functions can't take `var` parameters".
    - A `pure` function calling a non-pure native gives "pure functions can only call pure functions; `f` isn't pure".
    - A `par for` body calling one gives the existing `par for` message for non-pure calls.
  - **Codegen** (`codegen/tests/natives.rs`). Define Rust natives in the test file, compile a Kestrel program that declares and calls them with `compile_with`, and check the results through `print` capture (see `codegen/tests/common/mod.rs`):
    ```rust
    #[repr(C)] struct KStr { ptr: *const u8, len: i64 }
    #[repr(C)] struct V2 { x: f32, y: f32 }
    #[repr(C)] struct Rgba { r: f32, g: f32, b: f32, a: f32 }
    extern "C" fn k_twice(x: i32) -> i32 { x * 2 }
    extern "C" fn k_flip(b: u8) -> u8 { (b == 0) as u8 }
    extern "C" fn k_len(s: *const KStr) -> i32 { /* len, or 0 when ptr is null */ }
    extern "C" fn k_sum(v: *const V2) -> f32 { /* x + y */ }
    extern "C" fn k_swap(out: *mut V2, v: *const V2) { /* out = (v.y, v.x) */ }
    extern "C" fn k_alpha(c: *const Rgba) -> f32 { /* c.a */ }
    extern "C" fn k_tag(e: *const u8) -> i32 { /* the tag byte */ }
    ```
    Kestrel side, in the entry file:
    ```
    enum Dir {
        North
        East
        South
    }
    struct Rgba {
        r: float
        g: float
        b: float
        a: float = 1
    }
    native twice(x: int) -> int
    native flip(b: bool) -> bool
    native len(s: string) -> int
    native sum(v: vec2) -> float
    native swap(v: vec2) -> vec2
    native alpha(c: Rgba) -> float
    native tag(d: Dir) -> int
    main() {
        let n = 3
        print(twice(21), flip(true), len("abc{n}"), len(""), sum(vec2(1.5, 2)), swap(vec2(1, 2)), alpha(Rgba(1, 0, 0)), tag(Dir.South))
    }
    ```
    Expected output: `42 false 4 0 3.5 vec2(x: 2, y: 1) 1 2`.
- [ ] **Step 2: Run** the tests and see them fail.
- [ ] **Step 3: Implement.** Update `kestrel/godot/src` if anything it uses changed shape (it calls `JitProgram::compile`, which keeps its signature).
- [ ] **Step 4: Run** the workspace tests and both clippy runs. Commit "Add native functions".

### Task 3: Built-in modules in the driver, and struct constants

**Files:**
- Create: `driver/std/window.kes`, `driver/std/draw.kes`, `driver/std/input.kes`, and `driver/src/builtin.rs`
- Modify:
  - `driver/src/lib.rs`: `load` resolves built-in names; `Compiled.builtins`; the native-in-user-file error.
  - `check/src/body/consts.rs`: struct construction in constants; module constants are checked before the entry file's.
  - `check/src/body/structs.rs`: the `is_const_expr` call site.
  - `kestrel/godot/src/lib.rs`: refuse built-in modules.
- Test: `cli/tests/modules.rs` (or new error tests), `tests/errors/*.kes`, and `check/tests/` for constants

**The module sources** (write them exactly):

`driver/std/window.kes`:
```
// The window: opening it, the frame loop, and time.
pub native open(width: int, height: int, title: string)
pub native frame() -> bool
pub native close()
pub native delta() -> float
pub native time() -> float
pub native size() -> vec2
pub native fps() -> int
pub native screenshot(path: string)
```
`driver/std/draw.kes`:
```
// Drawing in window pixels: origin top-left, y down. The colour is always last.
pub struct Color {
    r: float
    g: float
    b: float
    a: float = 1
}

pub let WHITE = Color(1, 1, 1)
pub let BLACK = Color(0, 0, 0)
pub let GRAY = Color(0.745098, 0.745098, 0.745098)
pub let RED = Color(1, 0, 0)
pub let GREEN = Color(0, 1, 0)
pub let BLUE = Color(0, 0, 1)
pub let YELLOW = Color(1, 1, 0)
pub let ORANGE = Color(1, 0.647059, 0)
pub let PURPLE = Color(0.627451, 0.12549, 0.941176)
pub let TRANSPARENT = Color(1, 1, 1, 0)

pub native clear(color: Color)
pub native rect(pos: vec2, size: vec2, color: Color)
pub native rect_outline(pos: vec2, size: vec2, width: float, color: Color)
pub native circle(center: vec2, radius: float, color: Color)
pub native circle_outline(center: vec2, radius: float, width: float, color: Color)
pub native line(a: vec2, b: vec2, width: float, color: Color)
pub native triangle(a: vec2, b: vec2, c: vec2, color: Color)
pub native text(text: string, pos: vec2, size: float, color: Color)
pub native text_width(text: string, size: float) -> float
```
`driver/std/input.kes`: a comment line, then `pub enum Key` with one variant per line, in this order:
- `A` to `Z`
- `Digit0` to `Digit9`
- `Space Enter Escape Tab Backspace Shift Ctrl Alt Up Down Left Right`
- `F1` to `F12`
- `MouseLeft MouseRight MouseMiddle`

Then:
```
pub native held(key: Key) -> bool
pub native pressed(key: Key) -> bool
pub native released(key: Key) -> bool
pub native mouse() -> vec2
pub native wheel() -> float

// -1, 0 or 1, like Godot's Input.get_axis.
pub axis(negative: Key, positive: Key) -> float {
    var v = 0.0
    if held(positive) {
        v += 1
    }
    if held(negative) {
        v -= 1
    }
    v
}
```

**Rules:**
- **The built-in names are `window`, `draw` and `input`.**
  - In `load`, a `use` of one of them loads the embedded source with the virtual path `<builtin>/<name>.kes`. Diagnostics show that path, and hot reload sees no modification time, so it never reports a change.
  - A file `<name>.kes` in the directory of the file that wrote the `use` is the error "`draw` is a built-in module, so draw.kes can't be used as a module; rename the file", on the `use` clause.
- **`native` is only allowed in built-in modules.** A `native` declaration in any file the user wrote (entry or module) is the error "only Kestrel's built-in modules can declare native functions", reported by the driver at the declaration.
  - Task 2's `codegen/tests/natives.rs` declares natives in its entry file. If its harness (`codegen/tests/common`) goes through the driver, keep that test passing, for example by checking natives in a driver step the codegen harness doesn't use, and say how in the report.
- **`Compiled.builtins: Vec<&'static str>`** lists the built-in modules the program loaded, in load order.
- **The Godot add-on:** when `compiled.builtins` isn't empty, `load` fails and prints "window, draw and input can't be used inside Godot".
- **Struct constants.**
  - `is_const_expr` takes `&Tables` and accepts a construction (`ExprKind::Call` whose name is in `struct_by_name`, so `vec2`/`vec3` count) when every argument is constant.
  - `check_consts` checks constants with a qualified name (`module.NAME`) before the entry file's, keeping source order within each group. That lets `let BG = draw.BLACK` work in the entry file.
  - Module-to-module constant order is a follow-up; record it in Task 8.

- [ ] **Step 1: Failing tests.**
  - **Programs** (these need no window, because they never call a native): `tests/programs/builtin_modules.kes`:
    ```
    // expect: Color(r: 1, g: 0, b: 0, a: 1) Color(r: 0.2, g: 0.2, b: 0.2, a: 1)
    // expect: Key.Space vec2(x: 0, y: 1)
    use draw
    use { Color, RED from draw }
    use { Key from input }
    let BG = Color(0.2, 0.2, 0.2)
    let UP = vec2(0, 1)
    main() {
        print(RED, BG)
        print(Key.Space, UP)
    }
    ```
    If an enum value doesn't print as `Key.Space` (or a struct doesn't print as `Color(...)`), use what `print` actually produces for enums and module structs, and note it.
  - **Errors** (expectations at the end of each file):
    - `native f()` in a user entry file gives "only Kestrel's built-in modules can declare native functions".
    - `use draw` with a `draw.kes` next to the program gives the shadowing message. Use a directory fixture, as the existing module tests in `cli/tests/modules.rs` do.
    - `let X = Color(1, 2, sqrt(2))` is a valid constant (pure builtin plus construction). A constant `Color(1, 2, f())`, with `f` a user function, still gives "constants can only use literals, operators and earlier constants". Check what the message says today and keep it.
  - **Driver unit tests:** `Compiled.builtins` is `["draw", "input"]` for the program above. A module file named `window.kes` is refused.
- [ ] **Step 2: Run** the tests and see them fail.
- [ ] **Step 3: Implement.** Update the Godot add-on to refuse built-ins. Build it with `kestrel/godot/build.ps1` (run `powershell.exe -File build.ps1`) and run its headless test. Godot 4.7.2 Mono is at `C:\Users\sawye\Desktop\Godot_v4.7.2-stable_mono_win64\Godot_v4.7.2-stable_mono_win64\`; use the `_console.exe` with `--headless --path kestrel/godot/demo --script res://tests/test_bridge.gd`. All 25 checks must pass.
- [ ] **Step 4: Run** the workspace tests and both clippy runs. Commit "Embed the window, draw and input modules".

### Task 4: The natives in `kestrel-window`, real and headless

**Files:**
- Create in `window/src/`:
  - `state.rs` (the window state and the backend switch)
  - `natives_window.rs`, `natives_draw.rs`, `natives_input.rs`
  - `headless.rs` (printing a call)
  - `keys.rs` (the `Key` tag to raylib code table)
  - `abi.rs` (`KStr`, `V2`, `KColor`, and the reading helpers)
- Modify: `window/src/raylib.rs` (the functions used), `window/src/lib.rs` (`pub fn symbols() -> Vec<(&'static str, *const u8)>`), `runtime/src/panic.rs` (a public `fail`)

**Interfaces (Produces):**
- **The native symbols**, following the calling convention in the Architecture section. `KStr` is `{ ptr: *const u8, len: i64 }`, `V2` is `{ x, y: f32 }`, and `KColor` is `{ r, g, b, a: f32 }`.
  - `kestrel_window_open(width: i32, height: i32, title: *const KStr)`
  - `kestrel_window_frame() -> u8`
  - `kestrel_window_close()`
  - `kestrel_window_delta() -> f32`
  - `kestrel_window_time() -> f32`
  - `kestrel_window_size(out: *mut V2)`
  - `kestrel_window_fps() -> i32`
  - `kestrel_window_screenshot(path: *const KStr)`
  - `kestrel_draw_clear(c: *const KColor)`
  - `kestrel_draw_rect(pos: *const V2, size: *const V2, c: *const KColor)`
  - `kestrel_draw_rect_outline(pos, size, width: f32, c)`
  - `kestrel_draw_circle(center: *const V2, radius: f32, c)`
  - `kestrel_draw_circle_outline(center, radius: f32, width: f32, c)`
  - `kestrel_draw_line(a, b, width: f32, c)`
  - `kestrel_draw_triangle(a, b, c3, c)`
  - `kestrel_draw_text(text: *const KStr, pos, size: f32, c)`
  - `kestrel_draw_text_width(text, size: f32) -> f32`
  - `kestrel_input_held(key: *const u8) -> u8`, and the same for `kestrel_input_pressed` and `kestrel_input_released`
  - `kestrel_input_mouse(out: *mut V2)`
  - `kestrel_input_wheel() -> f32`
- `pub fn symbols() -> Vec<(&'static str, *const u8)>` lists all 22 symbols.
- `kestrel_runtime::panic::fail(msg: &str) -> !` becomes `pub`. It still runs the failure hook or exits with 101.

**Real backend** (raylib):
- **`open`:**
  1. `SetTraceLogLevel(LOG_ERROR)`.
  2. `SetConfigFlags(FLAG_WINDOW_RESIZABLE | FLAG_VSYNC_HINT | FLAG_MSAA_4X_HINT)`, plus `FLAG_WINDOW_HIDDEN` when the environment variable `KESTREL_HIDDEN=1` is set.
  3. `InitWindow` with a NUL-terminated copy of the title.
  4. `SetExitKey(KEY_NULL)`.
- **`frame()`:**
  1. The first call does `BeginDrawing` and returns `!WindowShouldClose()`.
  2. Later calls do `EndDrawing`. Then, if `WindowShouldClose()` or `close()` was called, they `CloseWindow`, mark the window closed and return false; otherwise they `BeginDrawing` and return true.
  3. Once the window is closed, every call returns false.
- **The queries:**
  - `delta` is `GetFrameTime`.
  - `time` is `GetTime() as f32`.
  - `size` is `GetScreenWidth`/`GetScreenHeight` as floats.
  - `fps` is `GetFPS`.
- **`screenshot`:** `rlDrawRenderBatchActive()`, `LoadImageFromScreen()`, `ExportImage(path)`, `UnloadImage`. That captures what has been drawn so far this frame.
- **Colours:** each channel is `(c.clamp(0, 1) * 255).round() as u8`.
- **Text:** `DrawTextEx(GetFontDefault(), …, spacing = size / 10)` and `MeasureTextEx(...).x`, with NUL-terminated copies of the text. An empty or null string draws nothing and measures 0.
- **Keys:** `keys.rs` maps each `Key` tag, in the `input.kes` order, to a raylib key code. `Shift`, `Ctrl` and `Alt` check both the left and right keys. The three mouse variants use `IsMouseButtonDown/Pressed/Released` with buttons 0, 1 and 2.
- **`wheel`:** `GetMouseWheelMove`.

**Headless backend:** used when `KESTREL_HEADLESS` holds a number `n`, read at `window.open`.
- No raylib calls are made.
- `frame()` returns true `n` times, then false.
- `delta()` is `1.0 / 60.0`, `time()` is `frames / 60`, `fps()` is 60, and `size()` is the opened size.
- Input reads as nothing held, pressed or released, `mouse()` is `(0, 0)` and `wheel()` is 0.
- `open`, `close`, `screenshot` and every `draw.*` call print one line through the runtime's print functions:
  - The line starts with the qualified name (`draw.circle`), then each argument after a space.
  - Numbers print as `print` does (Rust `{}` on the `f32`/`i32`).
  - A `vec2` prints as `vec2(x: 1, y: 2)`, and a colour as `Color(r: 1, g: 0.3, b: 0.3, a: 1)`.
  - Strings are quoted: `"hi"`.
  - Example: `draw.circle vec2(x: 400, y: 300) 20 Color(r: 1, g: 0.3, b: 0.3, a: 1)`.

**Errors**, through `kestrel_runtime::panic::fail`:
- "the window is already open" for a second `open`.
- "<name> was called before window.open", for example "draw.circle was called before window.open".
- "<name> was called after the window closed".

**Thread:** all state lives in one `static` behind a `Mutex` or a `thread_local`. Natives can't run in `par for` bodies, so only the program's thread calls them. A `thread_local` is fine.

- [ ] **Step 1: Failing unit tests.**
  - `keys_follow_input_kes`: `include_str!("../../driver/std/input.kes")`, extract the `Key` variant names in order, and check them against the names in `keys.rs`'s table, both in order and in count.
  - `color_conversion`: `(1, 0.3, -1, 2)` becomes `(255, 77, 0, 255)`.
  - `headless_lines`, on a spawned thread with `kestrel_runtime::print::capture_output()`:
    1. Set the headless state for 2 frames, then call the natives directly.
    2. Check the captured lines, including the quoting and the `vec2`/`Color` format.
    3. Check that `frame()` gives true, true, false.
  - `symbols_are_complete`: 22 entries, all names distinct, and every name starts with `kestrel_window_`, `kestrel_draw_` or `kestrel_input_`.
- [ ] **Step 2: Run** the tests and see them fail.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** the workspace tests and both clippy runs. Commit "Implement the window, draw and input natives".

### Task 5: Run and build window programs from the CLI

**Files:**
- Modify:
  - `cli/Cargo.toml` (depend on `kestrel-window`)
  - `cli/src/main.rs` (`run` uses `compile_with(..., &kestrel_window::symbols())`)
  - `cli/build.rs` (build `kestrel-window` as a staticlib too; export `KESTREL_WINDOW_LIB`; install it beside the binary)
  - `cli/src/link.rs` (pick the library; add the system libraries)
  - `cli/tests/programs.rs` (the `// env:` directive)
  - `cli/tests/relocated.rs` (window-library cases)
- Test:
  - `tests/programs/window_headless.kes`
  - `tests/programs/window_errors_before_open.kes`
  - `tests/programs/window_open_twice.kes`

**Rules:**
- **`cli/build.rs`** builds both libraries in one nested `cargo build -p kestrel-runtime -p kestrel-window --lib`. It exports `KESTREL_WINDOW_LIB` the way it exports `KESTREL_RUNTIME_LIB`, installs `kestrel_window.lib` beside the binary, and adds `rerun-if-changed` lines for `../window/src` and `../window/raylib`.
- **Linking:** when `compiled.builtins` isn't empty, `link` gets the window library instead of the runtime library, plus `opengl32.lib gdi32.lib winmm.lib user32.lib shell32.lib`. Change `link::link`'s signature to take which library to use, for example an enum `Lib::Runtime | Lib::Window`. The library lookup (beside the exe first, then the build path) works the same way for both.
- **The `// env: KEY=VALUE` directive** in `cli/tests/programs.rs`:
  - A program test may have several. Each one sets an environment variable for both the JIT run and the exe run.
  - A malformed one (no `=`) panics, like a bad expect line.
  - Parse the expectations before running.

- [ ] **Step 1: Failing tests.** `tests/programs/window_headless.kes`:
```
// env: KESTREL_HEADLESS=2
// expect: window.open 320 200 "hi"
// expect: frame 1 0.016666668 true
// expect: draw.clear Color(r: 0, g: 0, b: 0, a: 1)
// expect: draw.rect vec2(x: 1, y: 2) vec2(x: 3, y: 4) Color(r: 1, g: 0, b: 0, a: 1)
// expect: draw.text "n 1" vec2(x: 5, y: 5) 10 Color(r: 1, g: 1, b: 1, a: 0.5)
// expect: frame 2 0.016666668 true
// expect: draw.clear Color(r: 0, g: 0, b: 0, a: 1)
// expect: draw.rect vec2(x: 1, y: 2) vec2(x: 3, y: 4) Color(r: 1, g: 0, b: 0, a: 1)
// expect: draw.text "n 2" vec2(x: 5, y: 5) 10 Color(r: 1, g: 1, b: 1, a: 0.5)
// expect: done 2 vec2(x: 320, y: 200) 60 false vec2(x: 0, y: 0) 0 0
use window
use draw
use input
use { Color, RED from draw }
use { Key from input }

main() {
    window.open(320, 200, "hi")
    var n = 0
    while window.frame() {
        n += 1
        print("frame", n, window.delta(), input.held(Key.Space) == false)
        draw.clear(draw.BLACK)
        draw.rect(vec2(1, 2), vec2(3, 4), RED)
        draw.text("n {n}", vec2(5, 5), 10, Color(1, 1, 1, 0.5))
    }
    print("done", n, window.size(), window.fps(), input.pressed(Key.A), input.mouse(), input.wheel(), input.axis(Key.Left, Key.Right))
}
```
`tests/programs/window_errors_before_open.kes`:
```
// env: KESTREL_HEADLESS=1
// expect-exit: 101
// expect-stderr: draw.circle was called before window.open
use draw
main() {
    draw.circle(vec2(1, 1), 2, draw.RED)
}
```
`tests/programs/window_open_twice.kes`, the same shape, with "the window is already open".

Add `relocated` cases for the window library: it is found beside the exe, and a missing one gives a clear error.
- [ ] **Step 2: Run** `cargo test -p kestrel-cli` and see the failures. A missing `// env:` directive fails the parse first.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** the workspace tests and both clippy runs. Commit "Run and build window programs".

### Task 6: Interpolated text into natives uses a scratch arena

**Files:**
- `kir/src/lower/call.rs`, with `kir/src/lower/scopes.rs` as needed: the scratch arena.
- `runtime/src/start.rs`: the arena report.
- `window/src/state.rs`: the quiet flag.
- Tests: `kir/tests/natives.rs` (new), `cli/tests/text_memory.rs` (new).

**Goal:** the spec's promise that `draw.text("fps {window.fps()}", ...)` in the frame loop doesn't grow memory.

**Rules:**
- **The scratch arena.** In a call to a native, any argument that is directly an interpolated string (`ast::ExprKind::Interp`) is built in a scratch arena, which is freed right after the call.
  - Lower the call inside the same scope machinery `arena { }` uses (`kir/src/lower/scopes.rs`), so that `return`, `break` or `continue` inside an argument expression pops it. Read how `arena { }` lowers and reuse it; don't hand-roll `PushArena`/`PopArena` pairs that early exits skip.
  - A value-returning native call yields its scalar after the arena is popped.
  - A call with no interpolated argument is unchanged.
- **`KESTREL_HEADLESS_QUIET=1`** turns off the headless draw-call lines. It is read with `KESTREL_HEADLESS` at `window.open`.
- **`KESTREL_ARENA_REPORT=1`** makes the runtime print `kestrel: program arena <bytes> bytes` to stderr after `main` returns. Here `<bytes>` is the total size of the program arena's blocks; add a small `pub(crate)` accessor in `arena.rs` if one doesn't exist. Set the variable only in tests.

- [ ] **Step 1: Failing tests.**
  - **KIR:** a native call `len("x{n}")` lowers with the scratch arena around it (assert the statement shape), and `len("plain")` doesn't. Declare natives in the test's entry source, as Task 2's codegen test does.
  - **`cli/tests/text_memory.rs`:**
    1. Write a temporary program that runs `while window.frame() { n += 1; draw.text("n {n}", vec2(0, 0), 10, draw.WHITE) }`.
    2. Build it as an exe, then run it with `KESTREL_HEADLESS=1000` and with `KESTREL_HEADLESS=100000`, both with `KESTREL_HEADLESS_QUIET=1` and `KESTREL_ARENA_REPORT=1`.
    3. Parse the reported bytes. The two must be equal, or differ by less than 64 KiB.
    4. Run a control program that does `var last = ""` before the loop and `last = "n {n}"` inside it. It must grow by more than 1 MiB between the two runs, which shows the measurement works.
- [ ] **Step 2: Run** the tests and see them fail.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** the workspace tests and both clippy runs. Commit "Build interpolated native arguments in a scratch arena".

### Task 7: GPU smoke test

**Files:** `cli/tests/window_smoke.rs` (new)

- [ ] **Step 1: Write the test.**
  1. Write a temporary program: open a 64×64 window, then for 3 frames draw `clear(BLACK)` and `rect(vec2(16, 16), vec2(32, 32), RED)`. On the third frame, call `window.screenshot(<temp>/shot.png)` after drawing, then `window.close()`.
  2. Run it with `kestrel run` and `KESTREL_HIDDEN=1`.
  3. Check `kestrel_window::png_pixel(shot, 32, 32) == Some([255, 0, 0, 255])` and `png_pixel(shot, 2, 2) == Some([0, 0, 0, 255])`.
  4. If the hidden run's centre pixel isn't red, rerun without `KESTREL_HIDDEN` (a visible window) and assert on that.
  5. Print which mode passed, with `eprintln!`.
  6. Repeat with `kestrel build` and the exe.
- [ ] **Step 2: Run it**, and report which mode works on this machine. Commit "Smoke-test a real window".

### Task 8: The particles demo, docs and wrap-up

**Files:**
- Create: `kestrel/demos/particles/particles.kes`
- Modify:
  - the `kestrel/README.md` section on running it
  - `docs/superpowers/specs/2026-10-09-kestrel-syntax-design.md` (`native` in the grammar and declarations; the built-in modules)
  - `docs/superpowers/specs/2026-10-10-window-design.md` (the actual calling convention; headless strings are quoted; `KESTREL_HIDDEN`; `KESTREL_HEADLESS_QUIET`)
  - `docs/superpowers/plans/2026-10-09-core-2a-followups.md`
  - `docs/superpowers/plans/2026-10-10-handoff.md`

**The demo** (spec "Demo"): thousands of particles in a `list[Particle]`.
- `Particle` has `pos: vec2` and `vel: vec2`.
- `pure step(var self, dt: float, target: vec2)` accelerates toward `target`, with damping, and bounces off the window edges.
- Every frame:
  1. `par for p in parts { p.step(dt, target) }`
  2. Draw each particle as a 2×2 `draw.rect`.
  3. Draw a text overlay with the fps, the particle count and the update and draw times in milliseconds (measured with `window.time()` around each part).
- The mouse sets `target`. Space adds 1,000 particles at random positions (`randf_range`).
- Wrap the loop body in `arena { }` if it builds strings with `let`.

**Steps:**
- [ ] **Step 1: Write the demo.** Check that it runs headless: `KESTREL_HEADLESS=3 KESTREL_HEADLESS_QUIET=1 kestrel run demos/particles/particles.kes` finishes with exit 0.
- [ ] **Step 2: Measure.** Run it for real with `kestrel run --release` (and as a `--release` exe) at 10,000, 50,000 and 200,000 particles (change the starting count). Record, as medians over a few seconds:
  - the update time in ms
  - the draw time in ms
  - the fps
  
  Note where vsync caps the fps and where drawing, not Kestrel, is the bottleneck. Put the numbers in `kestrel/demos/particles/results.md`, honestly.
- [ ] **Step 3: Screenshot.** Take one real screenshot (temporarily add `window.screenshot(...)`, or use a copy of the demo) and look at it to confirm the particles and the text render. Don't commit the image.
- [ ] **Step 4: Docs.**
  - **Syntax spec:** the `native` declaration, with its grammar line `func := 'pure'? 'native'? IDENT ...` (no block when native), and the built-in modules `window`, `draw` and `input` (point to the window spec).
  - **Window spec:**
    - the real calling convention: a string is a pointer to its header, and an enum a pointer with the tag first;
    - headless strings are quoted;
    - `KESTREL_HIDDEN` and `KESTREL_HEADLESS_QUIET`.
  - **Follow-ups:**
    - Natives' runtime errors don't name the line.
    - Module-to-module constant order.
    - The window must move to the main thread for macOS.
    - Only Windows is built.
    - Textures and 3D come next.
    - Anything the reviews deferred.
  - **Handoff:** sub-project 4b part 1 is done, with the demo numbers; next is the 3D / textures spec. Update the test count.
- [ ] **Step 5: Run** the workspace tests and both clippy runs. Commit "Add the particles demo and document the window", then push.
