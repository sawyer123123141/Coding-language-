# Textures, 3D and the voxel viewer (sub-project 4b, part 2)

Status: syntax decided by the user on 2026-10-10. They took every pick in the proposal: a new `draw3d` module, a `Camera` struct set with `draw3d.camera(cam)`, `MeshData` built in Kestrel and turned into a GPU `Mesh`, Godot's clockwise winding, `Texture` and `Mesh` as owning values, `load_texture` failing with a runtime error, Godot's sprite names with a `Rect`, a bulk `draw.rects`, the small additions (`cross`, `ticks_usec`, 3D shapes), and sprite rotation later. Everything else is Claude's rulings, recorded here with what each costs if wrong.

**Scope:** textures and sprites in 2D, a bulk rectangle call, a 3D camera with meshes and a few shapes, and a demo that flies through the `bench/voxels3d` world. It also settles the window review's carry-in items: native array parameters, `pure native`, native error lines, GPU object lifetimes, a precise clock, and heap constants.

**Success:** the voxel viewer shows the 8×4×8-chunk world (463,749 faces) at 60 fps on the development machine. The particles demo draws 200,000 dots much faster than its current 30 ms. Every number is measured and reported honestly, including any that miss.

## What it looks like

```
use window
use draw
use draw3d
use input
use { Color, WHITE from draw }
use { Camera, MeshData from draw3d }
use { Key from input }

main() {
    window.open(1280, 720, "Kestrel 3D")
    let grass = draw.load_texture("grass.png")      // next to the program, like res://

    var data = MeshData()
    // Clockwise as seen from the camera, which looks along +z from z = -3.
    data.quad(vec3(0, 0, 0), vec3(1, 0, 0), vec3(1, 1, 0), vec3(0, 1, 0), Color(0.4, 0.8, 0.3))
    let wall = draw3d.make_mesh(data)               // copied to the GPU once

    var cam = Camera(pos: vec3(0.5, 0.5, -3), target: vec3(0.5, 0.5, 0))
    while window.frame() {
        cam.pos.x += input.axis(Key.A, Key.D) * 2 * window.delta()
        draw.clear(Color(0.5, 0.7, 1))
        draw3d.camera(cam)                          // 3D from here on
        draw3d.mesh(wall, vec3(0, 0, 0), WHITE)
        draw3d.cube(vec3(2, 0, 0), vec3(1, 1, 1), draw.RED)
        draw.texture(grass, vec2(10, 40), WHITE)    // a 2D call goes back to 2D, drawn on top
        draw.text("fps {window.fps()}", vec2(10, 10), 20, WHITE)
    }
}
```

3D is immediate mode: the program draws each mesh every frame, as `_draw()` does in Godot 2D. There is no scene tree.

## `draw`: textures, sprites and bulk rectangles

```
pub struct Rect { pos: vec2, size: vec2 }
pub struct Texture { id: GpuHandle, width: int, height: int }

draw.load_texture(path: string) -> Texture
draw.make_texture(width: int, height: int, pixels: [Color]) -> Texture
draw.texture(tex: Texture, pos: vec2, tint: Color)
draw.texture_rect(tex: Texture, pos: vec2, size: vec2, tint: Color)
draw.texture_region(tex: Texture, region: Rect, pos: vec2, size: vec2, tint: Color)
draw.rects(positions: [vec2], size: vec2, color: Color)
```

- `Rect` is Godot's `Rect2`: a top-left corner and a size.
- **Loading.**
  - `load_texture` reads a PNG, JPG or BMP file.
  - A relative path means the program's folder: the `.kes` file's folder under `kestrel run`, or the exe's folder for a built program. So a built game needs its images copied next to the exe.
  - A file that is missing or isn't an image is a runtime error naming the path and the line: "draw.load_texture couldn't read grass.png (looked for C:/game/grass.png): no such file on line 6".
  - `make_texture` builds a texture from pixels given row by row from the top-left; channels are clamped like any `Color`. Its errors: "draw.make_texture needs a width and height above 0, not 0 by 16" and "draw.make_texture needs width × height pixels: 16 by 16 is 256, not 255".
  - Both need the window open, because a texture lives on the GPU.
- **Drawing,** with Godot's names:
  - `texture` draws the whole texture at its natural size, top-left at `pos` (`draw_texture`).
  - `texture_rect` stretches it over `pos`/`size` (`draw_texture_rect`).
  - `texture_region` draws the `region` of the texture (in texture pixels, from the top-left) over `pos`/`size`, for sprite sheets and atlases (`draw_texture_rect_region`).
  - `tint` multiplies the texture's colours, like Godot's `modulate`; `WHITE` leaves them alone. The colour stays the last argument.
  - Textures are drawn with nearest filtering (crisp pixels, right for pixel art and voxel atlases), and coordinates outside a texture repeat it.
  - A negative width or height mirrors the image: in `size` it covers the rectangle the size spans from `pos`, flipped; in `region` it flips the part read.
- A texture's `width` and `height` are plain fields. `id` is the GPU object (see "GPU objects").
- **`rects`** draws one rectangle per position, all the same size and colour, in one call. It works exactly like calling `draw.rect` for each position in order (a `list[vec2]` is accepted too). It is built for the cost measured in the particles demo: one native call per rectangle, about 150–190 ns each, almost all of it in raylib's per-vertex batch and the driver.
- **Negative-size rectangles** (a window follow-up, fixed here): `rect`, `rect_outline` and `rects` with a negative width or height now cover the rectangle the size spans from `pos`, instead of drawing nothing.
- Rotation of sprites is not in this spec. It comes later with a 2D transform, like Godot's `draw_set_transform`.

## `draw3d`

A new built-in module, next to `window`, `draw` and `input`.

```
pub struct Camera {
    pos: vec3
    target: vec3
    up: vec3 = vec3(0, 1, 0)
    fov: float = 75
}

pub struct MeshData {
    vertices: list[vec3] = []
    colors: list[Color] = []
    uvs: list[vec2] = []
    indices: list[int] = []

    pure quad(var self, a: vec3, b: vec3, c: vec3, d: vec3, color: Color)
    pure triangle(var self, a: vec3, b: vec3, c: vec3, color: Color)
}

pub struct Mesh { id: GpuHandle, vertex_count: int, triangle_count: int }

draw3d.camera(cam: Camera)
draw3d.make_mesh(data: MeshData) -> Mesh
draw3d.mesh(m: Mesh, pos: vec3, tint: Color)
draw3d.mesh_textured(m: Mesh, tex: Texture, pos: vec3, tint: Color)
draw3d.cube(center: vec3, size: vec3, color: Color)
draw3d.sphere(center: vec3, radius: float, color: Color)
draw3d.line(a: vec3, b: vec3, color: Color)
```

- **Space:** y is up and the axes are right-handed, as in Godot 3D. Units are whatever the program uses (the voxel viewer uses one per block).
- **Camera.**
  - `fov` is the vertical field of view in degrees, 75 by default, like Godot's `Camera3D`.
  - The near and far planes are fixed at 0.05 and 4,000, Godot's defaults.
  - `draw3d.camera(cam)` reads the camera (a copy, so changing `cam` afterwards needs another call) and starts 3D drawing. The camera stays set, across frames too, until the next `draw3d.camera` call.
  - Its errors: "draw3d.camera needs pos and target to differ", "draw3d.camera needs an up that doesn't point along the view", and "draw3d.camera needs a fov between 0 and 180 degrees, not 200".
- **2D and 3D in one frame.**
  - `draw3d.camera` switches to 3D; the next `draw.*` call switches back to 2D on its own.
  - A later `draw3d.*` call switches back to 3D with the same camera.
  - A `draw3d` drawing call with no camera set yet is an error: "draw3d.mesh needs a camera; call draw3d.camera first".
  - So a frame can draw a 2D background, then the 3D world, then a 2D overlay, each on top of the last.
  - `draw.clear` clears depth as well as colour. 3D drawing tests depth; 2D drawing doesn't.
- **Meshes.**
  - `MeshData` is plain Kestrel. A program can build one anywhere, including in a `pure` function or a `par for` body, like any struct of lists.
  - `quad(a, b, c, d, color)` adds four vertices (`a b c d`) with `color`, and the two triangles `a b c` and `a c d`. `triangle` adds three vertices and one triangle. Neither adds uvs: a textured mesh adds one uv per vertex to `uvs` itself.
  - **Winding:** a triangle's front is the side its corners go clockwise from, as in Godot. Back faces aren't drawn. This is the order the `voxels3d` mesher already uses.
  - `colors` is empty (every vertex white) or has one colour per vertex. `uvs` is empty or has one per vertex: (0, 0) is the texture's top-left and (1, 1) its bottom-right, as in Godot.
  - `indices` lists three vertex numbers per triangle. With no indices, the vertices themselves are taken three at a time.
  - `make_mesh` copies the data to the GPU and keeps no copy of its own, so the `MeshData` can be freed right after. An empty `MeshData` (an all-air chunk) makes an empty mesh that draws nothing.
  - Its errors: "draw3d.make_mesh needs one color per vertex or none: 24 vertices, 23 colors" (and the same for uvs); "draw3d.make_mesh needs a multiple of 3 indices, not 7"; "draw3d.make_mesh got index 30, but there are only 24 vertices"; and, with no indices, "draw3d.make_mesh needs a multiple of 3 vertices when there are no indices, not 25".
  - `vertex_count` and `triangle_count` are plain fields.
- **Drawing meshes.**
  - `mesh(m, pos, tint)` draws `m` moved by `pos`, its vertex colours multiplied by `tint`.
  - `mesh_textured` does the same, multiplied by `tex` sampled at the uvs too. A mesh made without uvs is an error there: "draw3d.mesh_textured needs a mesh made with uvs".
  - Meshes have no rotation or scale yet; those come with a transform type.
- **Shapes.**
  - `cube` is a solid box centred on `center`, and `sphere` a solid ball.
  - `line` is one pixel wide. GPUs draw thick 3D lines poorly, so there is no width.
- **Lighting:** there is none. The colours are drawn as given, so a voxel mesher shades its faces in the vertex colours (top faces brightest, bottom darkest), as early Minecraft did. Normals come with lighting.
- Transparent colours blend, but nothing sorts triangles by distance, so overlapping see-through meshes can look wrong.

## `window`: a precise clock and paths

```
window.ticks_usec() -> i64
```

- `ticks_usec` is the microseconds since `window.open`, as an `i64`, like Godot's `Time.get_ticks_usec()`. `time()` stays a `float` for frame code; `ticks_usec` is for measuring, and stays exact for any run length.
- Headless, it is the frame count × 1,000,000 / 60, rounded down.
- `window.screenshot` resolves a relative path against the program's folder too, the same rule as `load_texture`. An absolute path is unchanged.

## Language additions

- **`a.cross(b)`** on `vec3`, GDScript's `Vector3.cross` (right-handed). It is `pure`, and the name isn't reserved, like the other vector methods.
- **Heap constants are built once** (window ruling N, decided here). An array, list or map constant, or one that holds such a value or a string built by interpolation, is built a single time before `main` starts. Every use reads that one value in place.
  - Indexing, `.len`, `for in`, a read-only argument and `print` copy nothing.
  - Storing the constant (`let palette = PALETTE`, a field, `list.add(PALETTE)`) copies it, as storing any other place does. A constant is never moved.
  - So the window spec's advice to wrap the loop in `arena { }` or copy the constant first is no longer needed: the palette that grew the arena by 4.78 MB over 100,000 frames now grows it by 0 bytes. A map constant used inside a hot function is no longer rebuilt at each call either.
  - Constants that allocate nothing (numbers, vectors, plain structs such as `draw.RED`) are unchanged: they are written in place where used.
  - A heap constant that no code uses is never built.
  - A runtime failure inside a heap constant's value (an overflow, say) now happens once, before `main`, with the constant's line.
- **`GpuHandle`** is a built-in type that only Kestrel's built-in modules can name. Elsewhere it is an error: "`GpuHandle` is only for Kestrel's built-in modules". Programs meet it only as the `id` field of `Texture` and `Mesh`. See the next section.

## GPU objects

`Texture` and `Mesh` hold a `GpuHandle`, so they are **owning**, like a struct that holds an `Arena`. In Godot terms they are like RefCounted resources that free themselves, except that Kestrel has exactly one owner.

- They move and never copy: `let b = a` moves `a`, and using `a` afterwards is the usual use-after-move error. Passing one to a function or a native passes it by pointer, as with any read-only argument, so drawing doesn't move it.
- The GPU memory is freed when the owner goes away: at the end of its scope, when it is overwritten, or when the list or struct holding it is dropped. Unlike an `Arena`, a place holding a texture or mesh may be overwritten (`c.mesh = draw3d.make_mesh(d)`); the old one is freed.
- A drop on any thread (including a `par for` pool thread) only queues the object. The window's thread frees the queue at the next `window.frame()`, so freeing is safe wherever the drop happens. A drop after the window has closed does nothing, because the GPU context has gone with it.
- A `GpuHandle` prints as its number (`Texture(id: 3, width: 16, height: 16)`). A handle of 0 means no object, as in a zero-filled array. Drawing one is an error: "draw.texture got an empty texture; make one with draw.load_texture or draw.make_texture" (and the same for meshes, with `draw3d.make_mesh`).
- Maps can't hold them, because maps can't hold owning values yet (a known gap).

## Natives

- **Array parameters (the carry-in ABI decision).**
  - A native may take `[T]` where `T` is a number, `bool`, `vec2`, `vec3`, or a struct of those (so `[Color]` works). A `list[T]` argument is accepted for a native's `[T]` parameter.
  - The native reads the first 16 bytes of the header, `{ptr, len}`, which an array and a list share. `ptr` is null when `len` is 0.
  - Elements are laid out as Kestrel lays them out (fields in order, natural alignment), so a `[vec3]` is packed float triples, raylib's vertex layout.
  - A native's struct parameter may also have fields of those array types, of `list[T]` with the same `T`, and of `GpuHandle`. That is how `MeshData`, `Texture` and `Mesh` reach the natives.
  - Cost if wrong: none for programs; a native that wants strings in its elements needs another rule later.
- **Results.** A native may now also return a struct of numbers, `bool`s, vectors and `GpuHandle`s (`Texture`, `Mesh`), through the result pointer that comes first, as `vec2` results already do.
- **`pure native` (carry-in rule).** A `pure native` can run on `par for` pool threads, where the window's state isn't available. So it must keep no state, touch no window, and be safe to call from any thread at once. Only built-in modules declare natives, so this is a rule for whoever writes them, checked in review. No native in this spec is `pure`.
- **Errors name the line (carry-in).**
  - Before each call from any of the program's own files (not a built-in module) to a built-in function that isn't `pure` (a native, or Kestrel code like `input.axis`), the generated code stores the call's line in a runtime slot: one store per call.
  - A native's runtime error ends with "on line N", like other runtime errors: "draw.circle was called before window.open on line 12".
  - Calls inside built-in modules don't store, so an error in a native that `input.axis` calls names the program's line, not one in `input.kes`.
  - `pure` built-in functions (`MeshData.quad`) don't store, because they can't reach a native that fails. That also keeps `par for` threads from all writing one shared slot.
  - The file isn't named, as for every runtime error so far.

## How it's built (Claude's rulings)

- **raylib:**
  - `rmodels.c` is now compiled (meshes, cubes, spheres), and JPG loading is switched on (`SUPPORT_FILEFORMAT_JPG`).
  - The extra raylib functions are declared by hand in `window/src/raylib.rs`, as before.
  - A window exe grows; the size is measured and recorded (it is about 812 KB now).
- **The GPU registry** (`kestrel-window`):
  - A handle is an `i64` from a counter that starts at 1 and is never reused, so headless lines and tests are deterministic.
  - A registry on the window's thread maps handles to raylib textures and meshes. The drop queue is a mutex-guarded list, drained in `window.frame()` and at close.
  - The drop function is a window-crate symbol that compiled code calls (registered with the JIT like the natives).
  - A program can only hold a `GpuHandle` through `draw` or `draw3d`, so it always links the window library.
- **Meshes over 65,536 vertices.** raylib's meshes use 16-bit indices. `make_mesh` splits a larger indexed mesh into parts of at most 65,536 vertices (triangles grouped in order, vertices renumbered per part) and draws the parts one after another. Kestrel code sees one `Mesh`. An unindexed mesh needs no split. Cost if wrong: a little more GPU memory, from vertices duplicated across parts.
- **Colours** are converted from `Color` (four floats) to raylib's four bytes when a mesh is made. Vertices and uvs are uploaded straight from the list memory, with no conversion.
- **Winding.** Kestrel's clockwise front faces are raylib's back faces, so mesh draws cull the other side. raylib's own cubes and spheres keep raylib's winding.
- **The 3D switch** keeps track of the current mode (2D, or 3D with a camera). Each `draw`/`draw3d` native switches if it has to (`BeginMode3D`/`EndMode3D`), and `window.frame()` and `window.screenshot` end a 3D pass first.
- **Bulk rectangles.** `draw.rects` fills one reusable vertex buffer with all its rectangles and draws it with a single GPU call, using raylib's default shader and the current 2D projection. It flushes raylib's batch first, so drawing order is kept. Cost if wrong: if raylib's internals change, this path needs updating with them.
- **Paths:**
  - The CLI tells the window crate the program's folder before running JIT code. A built exe uses its own folder.
  - The program-test harness gains `// asset: <file>`, which copies a file next to a test's built exe.
- **Hoisted constants:**
  - KIR gains global slots for heap constants and an init function that builds the used ones in dependency order (the order ruling C already computes), into a constants arena that lives as long as the program.
  - `main` calls the init first. The Godot add-on calls it on its worker thread after each load and hot reload, before the first call, because each add-on call runs in an arena freed afterwards.
  - Uses lower to reads of the slot, and lowering treats a constant as a place that is never moved, so storing it copies.
  - Codegen defines the slots as data in both the JIT and object files.
- **Error lines:** a runtime `i32` slot written with a plain store, and a helper that natives use to fail with "... on line N". Only the window's thread runs non-`pure` builtins, so there is no race.
- **Godot add-on:** it refuses `draw3d` too. The message becomes "window, draw, draw3d and input can't be used inside Godot". Hoisted constants work there through the init call.
- **Headless lines** for every new native, printed like `print` prints the values:
  - `draw.texture Texture(id: 1, width: 16, height: 16) vec2(x: 10, y: 40) Color(r: 1, g: 1, b: 1, a: 1)`
  - `draw3d.camera Camera(pos: vec3(x: 0, y: 0, z: -3), target: vec3(x: 0, y: 0, z: 0), up: vec3(x: 0, y: 1, z: 0), fov: 75)`
  - Arrays print as their length, not their elements: `draw.rects 50000 vec2(x: 2, y: 2) Color(r: 1, g: 0.65, b: 0.2, a: 1)` and `draw3d.make_mesh MeshData(vertices: 24, colors: 24, uvs: 0, indices: 36)`.
  - Headless `load_texture` still reads the file, for its size and its errors, without a GPU. `make_texture` checks its pixels. Handles count from 1, as in a real window.

## Testing

- **Program tests** (headless, JIT and exe):
  - every new native's line;
  - `MeshData.quad` and `triangle` counts;
  - `cross`;
  - `ticks_usec`;
  - hoisted constants: in-place reads, copies when stored, a constant that is never moved, map and list constants, a constant used in a `par for`;
  - each native error, with its line (`// expect-stderr:`);
  - `load_texture` of a PNG beside the test (`// asset:`).
- **Error tests:** `GpuHandle` named in a program; using a texture after moving it; copying a mesh into two places; a texture in a map.
- **Driver tests** with stand-in built-in modules: the new native parameter and result rules (`[T]` with each allowed `T`, `[string]` refused, a `list[T]` argument for a `[T]` parameter, struct results).
- **Window crate unit tests:**
  - mesh validation and splitting (a 70,000-vertex mesh splits into two parts that draw the same triangles);
  - colour conversion;
  - the drop queue (a drop from another thread is freed at the next frame, a drop after close is ignored);
  - camera validation.
- **Memory test:** the palette program from ruling N grows the program arena by 0 bytes over 100,000 frames (`KESTREL_ARENA_REPORT`).
- **GPU smoke test** (`cli/tests/window_smoke.rs`, a hidden window as now, JIT and exe). It checks pixels from:
  - `make_texture` and `load_texture` (a PNG the test writes), with `texture`, `texture_rect`, `texture_region` and a mirrored region;
  - `draw.rects` and a negative-size `draw.rect`;
  - a clockwise quad facing the camera, which is drawn, and the same quad wound the other way, which isn't (this checks the winding rule);
  - `mesh_textured`;
  - `cube`;
  - a 2D overlay drawn over 3D.
- **Godot:** the headless test checks the new refusal message, and that a program with a heap constant still works through the add-on.

## Demos

**Particles** (`kestrel/demos/particles`). After the `par for`, a loop copies each dot's position into a `list[vec2]`, and `draw.rects` draws them in one call. A constant `BULK` switches back to one `draw.rect` per dot, so `results.md` can compare the two. The copy counts as drawing time. The table is re-measured at 10,000, 50,000, 100,000 and 200,000 dots, plus higher counts if 200,000 fits in a frame. The old per-call numbers stay as the baseline.

**The voxel viewer** (`kestrel/demos/voxels/`, new):
- `world.kes` holds the `voxels3d` noise and mesher, changed to fill a `MeshData`. Each face is coloured by its height band and shaded by the way it faces (top 1.0, sides 0.8 and 0.7, bottom 0.5).
- `voxels.kes` opens a 1280×720 window. Inside an `arena { }`, a `par for` builds every chunk's `MeshData`, a loop makes a `Mesh` from each, and the arena then frees all the CPU data.
- The frame loop flies the camera: WASD to move, Space and Shift to go up and down, the right mouse button held and dragged to look, and the wheel to change speed. It draws every chunk mesh and a text overlay with the fps, the frame time and the face count.
- It prints a line once a second, like the particles demo, and a line at start with the world build time and the upload time (from `ticks_usec`).
- A constant `WIDTH` sets the world size: 8 chunks across, as in the benchmark, and larger for a stress run.
- **Result:** `results.md` records the build (`par for`) and upload times, the frame time and fps at `WIDTH` 8 and 16, the exe size, and what limits it, honestly.

## Build order

Three plans, each run subagent-driven on Sonnet:

1. **Compiler:** `cross`; hoisted heap constants (with the Godot init call and the memory test); native array parameters and struct results; `GpuHandle` (checker, owning, drop call, printing); native error lines; the asset directive.
2. **Window:**
   - the raylib build;
   - the GPU registry and drop queue;
   - textures and sprites;
   - `draw.rects` and negative-size rectangles;
   - `draw3d` (camera, the 2D/3D switch, shapes);
   - `MeshData` and meshes;
   - `ticks_usec` and paths;
   - headless lines, smoke tests, the Godot refusal;
   - the particles demo, re-measured.
3. **The voxel viewer:** the demo and its `results.md`, then the docs: the handoff, the syntax spec (its old `let tex = load("grass.png") else { return }` example goes), the window spec's frame-loop advice, and the follow-ups.

## Not in this spec

- Lighting, normals, fog and shaders.
- Rotation and scale of meshes (they wait for a transform type), and rotation of sprites (it waits for a 2D transform).
- Mouse capture: the viewer looks around by dragging.
- Model files (glTF, OBJ), and updating a texture's pixels after it is made.
- Texture filtering options and mipmaps.
- Transparency sorting.
- Audio, gamepads, fonts, a windowed (no console) exe.
- Linux and macOS (the window must move to the main thread for macOS), and building raylib once instead of twice.
- `draw`, `draw3d` or `MeshData` inside the Godot add-on.
