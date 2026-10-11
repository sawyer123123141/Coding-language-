//! Headless mode: no window, and each call prints one line instead of drawing.
//!
//! A line is the qualified name (`draw.circle`), then each argument after a
//! space, printed the way `print` prints it: numbers as Rust's `{}` does,
//! a `vec2` as `vec2(x: 1, y: 2)`, a colour as `Color(r: 1, g: 0.3, b: 0.3, a: 1)`,
//! a `vec3` as `vec3(x: 0, y: 0, z: -3)`, a `Camera` as
//! `Camera(pos: vec3(x: 0, y: 0, z: -3), target: vec3(x: 0, y: 0, z: 0), up: vec3(x: 0, y: 1, z: 0), fov: 75)`,
//! a `Rect` as `Rect(pos: vec2(x: 0, y: 0), size: vec2(x: 4, y: 2))`, a `Texture` as
//! `Texture(id: 1, width: 4, height: 4)`, a `Mesh` as `Mesh(id: 1, vertex_count: 4, triangle_count: 2)`,
//! a `MeshData` as `MeshData(vertices: 4, colors: 4, uvs: 0, indices: 6)` (its lists as their
//! lengths), an array as its length and a string in double quotes. The lines go through the runtime's print functions, so they stay
//! in order with the program's own `print` output.
//!
//! With `KESTREL_HEADLESS_QUIET=1` no line is printed at all: a program that
//! draws thousands of times a frame can run headless without flooding its output.

use std::cell::Cell;

use kestrel_runtime::print::{
    kestrel_rt_print_f32, kestrel_rt_print_i64, kestrel_rt_print_newline, kestrel_rt_print_space, kestrel_rt_print_str,
};

use crate::abi::{KCamera, KColor, KMesh, KRect, KTexture, V2, V3};

/// Frames per second the headless clock pretends to run at.
pub const FPS: i32 = 60;

/// One argument of a printed call.
#[derive(Clone, Copy)]
pub enum Arg<'a> {
    Int(i32),
    Float(f32),
    Str(&'a [u8]),
    Vec2(V2),
    Vec3(V3),
    Color(KColor),
    /// An array argument: it prints as its length.
    Count(i64),
    Rect(KRect),
    Texture(KTexture),
    Camera(KCamera),
    Mesh(KMesh),
    /// A `MeshData` argument: it prints its lists as their lengths.
    MeshData { vertices: usize, colors: usize, uvs: usize, indices: usize },
}

thread_local! {
    /// Set when the headless window opens; the window and its lines belong to one thread.
    static QUIET: Cell<bool> = const { Cell::new(false) };
}

/// Turns the lines off (or on) for this thread's window.
pub fn set_quiet(quiet: bool) {
    QUIET.set(quiet);
}

/// Prints `name` and its arguments on one line, unless the window is quiet.
pub fn line(name: &str, args: &[Arg]) {
    if QUIET.get() {
        return;
    }
    text(name.as_bytes());
    for arg in args {
        kestrel_rt_print_space();
        match *arg {
            Arg::Int(v) => kestrel_rt_print_i64(i64::from(v)),
            Arg::Float(v) => kestrel_rt_print_f32(v),
            Arg::Str(bytes) => {
                text(b"\"");
                text(bytes);
                text(b"\"");
            }
            Arg::Vec2(v) => vec2(v),
            Arg::Vec3(v) => vec3(v),
            Arg::Color(c) => color(c),
            Arg::Count(n) => kestrel_rt_print_i64(n),
            Arg::Rect(r) => rect(r),
            Arg::Texture(t) => texture(t),
            Arg::Camera(c) => camera(c),
            Arg::Mesh(m) => mesh(m),
            Arg::MeshData { vertices, colors, uvs, indices } => mesh_data([vertices, colors, uvs, indices]),
        }
    }
    kestrel_rt_print_newline();
}

fn text(bytes: &[u8]) {
    // SAFETY: `bytes` is a live slice. The runtime ignores a zero length.
    unsafe { kestrel_rt_print_str(bytes.as_ptr(), bytes.len() as i64) }
}

fn field(label: &[u8], v: f32) {
    text(label);
    kestrel_rt_print_f32(v);
}

fn vec2(v: V2) {
    field(b"vec2(x: ", v.x);
    field(b", y: ", v.y);
    text(b")");
}

fn vec3(v: V3) {
    field(b"vec3(x: ", v.x);
    field(b", y: ", v.y);
    field(b", z: ", v.z);
    text(b")");
}

fn camera(c: KCamera) {
    text(b"Camera(pos: ");
    vec3(c.pos);
    text(b", target: ");
    vec3(c.target);
    text(b", up: ");
    vec3(c.up);
    field(b", fov: ", c.fov);
    text(b")");
}

/// A float as `print` prints it (`200`, `0.5`, `NaN`): what an error message shows for a number.
/// Same as `Arg::Float` in a line, which a test checks.
pub fn float_text(v: f32) -> String {
    v.to_string()
}

fn rect(r: KRect) {
    text(b"Rect(pos: ");
    vec2(r.pos);
    text(b", size: ");
    vec2(r.size);
    text(b")");
}

fn texture(t: KTexture) {
    text(b"Texture(id: ");
    kestrel_rt_print_i64(t.id);
    text(b", width: ");
    kestrel_rt_print_i64(i64::from(t.width));
    text(b", height: ");
    kestrel_rt_print_i64(i64::from(t.height));
    text(b")");
}

fn mesh(m: KMesh) {
    text(b"Mesh(id: ");
    kestrel_rt_print_i64(m.id);
    text(b", vertex_count: ");
    kestrel_rt_print_i64(i64::from(m.vertex_count));
    text(b", triangle_count: ");
    kestrel_rt_print_i64(i64::from(m.triangle_count));
    text(b")");
}

/// The lengths of a `MeshData`'s four lists, in field order.
fn mesh_data(lengths: [usize; 4]) {
    let labels: [&[u8]; 4] = [b"MeshData(vertices: ", b", colors: ", b", uvs: ", b", indices: "];
    for (label, n) in labels.into_iter().zip(lengths) {
        text(label);
        kestrel_rt_print_i64(n as i64);
    }
    text(b")");
}

fn color(c: KColor) {
    field(b"Color(r: ", c.r);
    field(b", g: ", c.g);
    field(b", b: ", c.b);
    field(b", a: ", c.a);
    text(b")");
}

/// How wide `draw.text_width` reports a text in headless mode, where there is
/// no font to measure: half the size for each character. It only has to be
/// the same on every run.
pub fn text_width(text: &[u8], size: f32) -> f32 {
    String::from_utf8_lossy(text).chars().count() as f32 * size * 0.5
}

#[cfg(test)]
mod tests {
    use super::*;
    use kestrel_runtime::print::{capture_output, take_output};

    /// Runs `body` on a fresh thread, because the print sink is thread-local.
    fn captured(body: impl FnOnce() + Send + 'static) -> String {
        std::thread::spawn(move || {
            capture_output();
            body();
            String::from_utf8(take_output()).unwrap()
        })
        .join()
        .unwrap()
    }

    #[test]
    fn a_line_prints_the_name_then_each_argument() {
        let out = captured(|| {
            line(
                "draw.circle",
                &[Arg::Vec2(V2 { x: 400.0, y: 300.0 }), Arg::Float(20.0), Arg::Color(KColor { r: 1.0, g: 0.3, b: 0.3, a: 1.0 })],
            );
            line("window.open", &[Arg::Int(320), Arg::Int(-1), Arg::Str(b"hi")]);
            line("window.close", &[]);
            line("draw.text", &[Arg::Str(b""), Arg::Float(0.016666668)]);
        });
        assert_eq!(
            out,
            "draw.circle vec2(x: 400, y: 300) 20 Color(r: 1, g: 0.3, b: 0.3, a: 1)\n\
             window.open 320 -1 \"hi\"\n\
             window.close\n\
             draw.text \"\" 0.016666668\n"
        );
    }

    #[test]
    fn texture_arguments_print_the_way_print_prints_them() {
        let out = captured(|| {
            let texture = KTexture { id: 3, width: 16, height: 8 };
            let region = KRect { pos: V2 { x: 0.0, y: 2.5 }, size: V2 { x: -4.0, y: 4.0 } };
            line("draw.texture_region", &[Arg::Texture(texture), Arg::Rect(region), Arg::Count(256)]);
            line("draw.texture", &[Arg::Texture(KTexture { id: 0, width: 0, height: 0 })]);
        });
        assert_eq!(
            out,
            "draw.texture_region Texture(id: 3, width: 16, height: 8) Rect(pos: vec2(x: 0, y: 2.5), size: vec2(x: -4, y: 4)) 256\n\
             draw.texture Texture(id: 0, width: 0, height: 0)\n"
        );
    }

    #[test]
    fn camera_and_vec3_arguments_print_the_way_print_prints_them() {
        let out = captured(|| {
            let (origin, up) = (V3 { x: 0.0, y: 0.0, z: 0.0 }, V3 { x: 0.0, y: 1.0, z: 0.0 });
            let camera = KCamera { pos: V3 { x: 0.0, y: 0.5, z: -3.0 }, target: origin, up, fov: 75.0 };
            line("draw3d.camera", &[Arg::Camera(camera)]);
            line("draw3d.line", &[Arg::Vec3(origin), Arg::Vec3(V3 { x: 1.5, y: -2.0, z: 3.0 })]);
        });
        assert_eq!(
            out,
            "draw3d.camera Camera(pos: vec3(x: 0, y: 0.5, z: -3), target: vec3(x: 0, y: 0, z: 0), \
             up: vec3(x: 0, y: 1, z: 0), fov: 75)\n\
             draw3d.line vec3(x: 0, y: 0, z: 0) vec3(x: 1.5, y: -2, z: 3)\n"
        );
    }

    #[test]
    fn mesh_arguments_print_the_way_print_prints_them_with_lists_as_lengths() {
        let out = captured(|| {
            let data = Arg::MeshData { vertices: 24, colors: 24, uvs: 0, indices: 36 };
            line("draw3d.make_mesh", &[data]);
            let mesh = KMesh { id: 7, vertex_count: 4, triangle_count: 2 };
            line("draw3d.mesh", &[Arg::Mesh(mesh), Arg::Vec3(V3 { x: 1.0, y: 0.0, z: -2.5 })]);
        });
        assert_eq!(
            out,
            "draw3d.make_mesh MeshData(vertices: 24, colors: 24, uvs: 0, indices: 36)\n\
             draw3d.mesh Mesh(id: 7, vertex_count: 4, triangle_count: 2) vec3(x: 1, y: 0, z: -2.5)\n"
        );
    }

    #[test]
    fn float_text_is_what_a_line_prints_for_a_float() {
        for v in [200.0, 0.5, -0.0, 1e-7, 3.4e38, 0.016666668, f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            let printed = captured(move || line("x", &[Arg::Float(v)]));
            assert_eq!(printed, format!("x {}\n", float_text(v)), "{v}");
        }
        assert_eq!(float_text(200.0), "200");
        assert_eq!(float_text(0.5), "0.5");
    }

    #[test]
    fn a_quiet_window_prints_no_lines() {
        let out = captured(|| {
            set_quiet(true);
            line("draw.clear", &[]);
            line("window.close", &[]);
            set_quiet(false);
            line("draw.clear", &[]);
        });
        assert_eq!(out, "draw.clear\n");
    }

    #[test]
    fn the_headless_text_width_counts_characters() {
        assert_eq!(text_width(b"", 10.0), 0.0);
        assert_eq!(text_width(b"abcde", 10.0), 25.0);
        assert_eq!(text_width("héllo".as_bytes(), 10.0), 25.0, "characters, not bytes");
    }
}
