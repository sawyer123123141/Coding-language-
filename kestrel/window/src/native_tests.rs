//! Tests that call the natives the way compiled code does, in headless mode.

use std::collections::HashSet;
use std::path::Path;

use kestrel_runtime::print::{capture_output, take_output};

use crate::abi::{KColor, KRect, KSeq, KStr, KTexture, V2};
use crate::gpu;
use crate::natives_draw::*;
use crate::natives_input::*;
use crate::natives_texture::*;
use crate::natives_window::*;
use crate::state::{Mode, Settings};
use crate::symbols;

fn kstr(s: &str) -> KStr {
    KStr { ptr: s.as_ptr(), len: s.len() as i64 }
}

const NO_STRING: KStr = KStr { ptr: std::ptr::null(), len: 0 };

fn v2(x: f32, y: f32) -> V2 {
    V2 { x, y }
}

fn rgba(r: f32, g: f32, b: f32, a: f32) -> KColor {
    KColor { r, g, b, a }
}

/// Runs `body` on a fresh thread (the print sink and the window state are
/// per thread) and gives the lines it printed, and what it returned.
fn headless_run<T: Send + 'static>(body: impl FnOnce() -> T + Send + 'static) -> (Vec<String>, T) {
    let (out, result) = std::thread::spawn(move || {
        capture_output();
        let result = body();
        (String::from_utf8(take_output()).unwrap(), result)
    })
    .join()
    .unwrap();
    (out.lines().map(str::to_string).collect(), result)
}

/// `headless_run` for a body that returns nothing: just the lines.
fn headless_lines_of(body: impl FnOnce() + Send + 'static) -> Vec<String> {
    headless_run(body).0
}

fn headless(frames: u64) -> Settings {
    Settings { headless: Some(frames), hidden: false, quiet: false }
}

#[test]
fn headless_lines() {
    let lines = headless_lines_of(|| {
        let (title, shot, hi) = (kstr("hi"), kstr("shot.png"), kstr("hi"));
        let (white, red, black) = (rgba(1.0, 1.0, 1.0, 0.5), rgba(1.0, 0.3, 0.3, 1.0), rgba(0.0, 0.0, 0.0, 1.0));
        // SAFETY: every pointer is to a live local, as compiled code passes them.
        unsafe {
            open_with(&headless(2), 320, 200, &title);
            assert_eq!(kestrel_window_frame(), 1);
            kestrel_draw_clear(&black);
            kestrel_draw_rect(&v2(1.0, 2.0), &v2(3.0, 4.0), &red);
            kestrel_draw_rect_outline(&v2(1.0, 2.0), &v2(3.5, 4.0), 2.0, &white);
            kestrel_draw_circle(&v2(400.0, 300.0), 20.0, &red);
            kestrel_draw_circle_outline(&v2(0.5, 0.25), 8.0, 1.5, &red);
            kestrel_draw_line(&v2(0.0, 0.0), &v2(10.0, -10.0), 3.0, &white);
            kestrel_draw_triangle(&v2(0.0, 0.0), &v2(5.0, 0.0), &v2(0.0, 5.0), &black);
            kestrel_draw_text(&hi, &v2(5.0, 5.0), 10.0, &white);
            kestrel_draw_text(&NO_STRING, &v2(5.0, 5.0), 10.0, &white);
            assert_eq!(kestrel_draw_text_width(&hi, 10.0), 10.0);
            assert_eq!(kestrel_draw_text_width(&NO_STRING, 10.0), 0.0);

            assert_eq!(kestrel_window_frame(), 1);
            kestrel_window_screenshot(&shot);
            assert_eq!(kestrel_window_frame(), 0, "two frames, then false");
            assert_eq!(kestrel_window_frame(), 0, "and it stays false");
        }
    });
    assert_eq!(
        lines,
        [
            "window.open 320 200 \"hi\"",
            "draw.clear Color(r: 0, g: 0, b: 0, a: 1)",
            "draw.rect vec2(x: 1, y: 2) vec2(x: 3, y: 4) Color(r: 1, g: 0.3, b: 0.3, a: 1)",
            "draw.rect_outline vec2(x: 1, y: 2) vec2(x: 3.5, y: 4) 2 Color(r: 1, g: 1, b: 1, a: 0.5)",
            "draw.circle vec2(x: 400, y: 300) 20 Color(r: 1, g: 0.3, b: 0.3, a: 1)",
            "draw.circle_outline vec2(x: 0.5, y: 0.25) 8 1.5 Color(r: 1, g: 0.3, b: 0.3, a: 1)",
            "draw.line vec2(x: 0, y: 0) vec2(x: 10, y: -10) 3 Color(r: 1, g: 1, b: 1, a: 0.5)",
            "draw.triangle vec2(x: 0, y: 0) vec2(x: 5, y: 0) vec2(x: 0, y: 5) Color(r: 0, g: 0, b: 0, a: 1)",
            "draw.text \"hi\" vec2(x: 5, y: 5) 10 Color(r: 1, g: 1, b: 1, a: 0.5)",
            "draw.text \"\" vec2(x: 5, y: 5) 10 Color(r: 1, g: 1, b: 1, a: 0.5)",
            "draw.text_width \"hi\" 10",
            "draw.text_width \"\" 10",
            "window.screenshot \"shot.png\"",
        ]
    );
}

#[test]
fn headless_queries_print_nothing_and_read_as_idle() {
    let lines = headless_lines_of(|| {
        let title = kstr("t");
        // SAFETY: every pointer is to a live local.
        unsafe {
            open_with(&headless(3), 640, 480, &title);
            assert_eq!(kestrel_window_frame(), 1);
            assert_eq!(kestrel_window_delta(), 1.0 / 60.0);
            assert_eq!(kestrel_window_time(), 1.0 / 60.0);
            assert_eq!(kestrel_window_fps(), 60);
            assert_eq!(kestrel_window_frame(), 1);
            assert_eq!(kestrel_window_time(), 2.0 / 60.0);
            let mut size = v2(-1.0, -1.0);
            kestrel_window_size(&mut size);
            assert_eq!(size, v2(640.0, 480.0));

            let (a, space, mouse_middle) = (0u8, 36u8, 62u8);
            for key in [a, space, mouse_middle, 200] {
                assert_eq!(kestrel_input_held(&key), 0);
                assert_eq!(kestrel_input_pressed(&key), 0);
                assert_eq!(kestrel_input_released(&key), 0);
            }
            let mut at = v2(9.0, 9.0);
            kestrel_input_mouse(&mut at);
            assert_eq!(at, v2(0.0, 0.0));
            assert_eq!(kestrel_input_wheel(), 0.0);

            // The same after the last frame: queries keep the final values.
            assert_eq!(kestrel_window_frame(), 1);
            assert_eq!(kestrel_window_frame(), 0);
            assert_eq!(kestrel_window_time(), 3.0 / 60.0);
            assert_eq!(kestrel_window_fps(), 60);
            kestrel_window_size(&mut size);
            assert_eq!(size, v2(640.0, 480.0));
            assert_eq!(kestrel_input_held(&a), 0);
            kestrel_input_mouse(&mut at);
            assert_eq!(at, v2(0.0, 0.0));
        }
    });
    assert_eq!(lines, ["window.open 640 480 \"t\""]);
}

#[test]
fn ticks_usec_counts_microseconds_since_open_and_prints_nothing() {
    let lines = headless_lines_of(|| {
        let title = kstr("t");
        // SAFETY: every pointer is to a live local.
        unsafe {
            open_with(&headless(3), 10, 10, &title);
            assert_eq!(kestrel_window_ticks_usec(), 0);
            let mut seen = Vec::new();
            while kestrel_window_frame() == 1 {
                seen.push(kestrel_window_ticks_usec());
            }
            assert_eq!(seen, [16_666, 33_333, 50_000], "a sixtieth of a second a frame, rounded down");
            assert_eq!(kestrel_window_ticks_usec(), 50_000, "after the window closes it keeps its last value");
        }
    });
    assert_eq!(lines, ["window.open 10 10 \"t\""]);
}

#[test]
fn a_quiet_headless_window_prints_nothing_but_still_behaves() {
    let lines = headless_lines_of(|| {
        let (title, shot) = (kstr("t"), kstr("shot.png"));
        let black = rgba(0.0, 0.0, 0.0, 1.0);
        // SAFETY: every pointer is to a live local.
        unsafe {
            open_with(&Settings { quiet: true, ..headless(2) }, 10, 10, &title);
            assert_eq!(kestrel_window_frame(), 1);
            kestrel_draw_clear(&black);
            kestrel_draw_text(&title, &v2(0.0, 0.0), 10.0, &black);
            assert_eq!(kestrel_draw_text_width(&title, 10.0), 5.0, "the width is still measured");
            kestrel_window_screenshot(&shot);
            assert_eq!(kestrel_window_frame(), 1);
            kestrel_window_close();
            assert_eq!(kestrel_window_frame(), 0, "and the frames still run out");
        }
    });
    assert_eq!(lines, Vec::<String>::new());
}

#[test]
fn closing_prints_and_ends_the_frame_loop() {
    let lines = headless_lines_of(|| {
        let title = kstr("t");
        // SAFETY: every pointer is to a live local.
        unsafe {
            open_with(&headless(100), 10, 10, &title);
            assert_eq!(kestrel_window_frame(), 1);
            kestrel_window_close();
            // Still drawable until the frame ends.
            kestrel_draw_clear(&rgba(0.0, 0.0, 0.0, 1.0));
            assert_eq!(kestrel_window_frame(), 0);
            kestrel_window_close(); // does nothing once closed
        }
    });
    assert_eq!(lines, ["window.open 10 10 \"t\"", "window.close", "draw.clear Color(r: 0, g: 0, b: 0, a: 1)"]);
}

#[test]
fn symbols_are_complete() {
    let table = symbols();
    assert_eq!(table.len(), 28);
    let names: HashSet<&str> = table.iter().map(|&(name, _)| name).collect();
    assert_eq!(names.len(), 28, "names are distinct");
    let addresses: HashSet<usize> = table.iter().map(|&(_, address)| address as usize).collect();
    assert_eq!(addresses.len(), 28, "each name has its own function");
    for &(name, address) in &table {
        assert!(
            name.starts_with("kestrel_window_") || name.starts_with("kestrel_draw_") || name.starts_with("kestrel_input_"),
            "{name}"
        );
        assert!(!address.is_null(), "{name}");
    }
}

#[test]
fn symbols_cover_every_native_the_modules_declare() {
    let mut declared = HashSet::new();
    for (module, source) in [
        ("window", include_str!("../../driver/std/window.kes")),
        ("draw", include_str!("../../driver/std/draw.kes")),
        ("input", include_str!("../../driver/std/input.kes")),
    ] {
        for line in source.lines() {
            // `pub native f(...)` or `pub pure native f(...)`.
            if let Some(rest) = line.strip_prefix("pub native ").or_else(|| line.strip_prefix("pub pure native ")) {
                let name = rest.split('(').next().unwrap();
                declared.insert(format!("kestrel_{module}_{name}"));
            }
        }
    }
    let provided: HashSet<String> = symbols().iter().map(|&(name, _)| name.to_string()).collect();
    assert_eq!(declared.len(), 28);
    assert_eq!(provided, declared);
}

// --- Textures ---

const NO_TEXTURE: KTexture = KTexture { id: 0, width: 0, height: 0 };

/// The test image, by an absolute path: unit tests have no program folder.
fn checker_path() -> String {
    format!("{}/../tests/programs/assets/checker.png", env!("CARGO_MANIFEST_DIR"))
}

/// An array header over `colors`, as compiled code passes a `[Color]`.
fn seq(colors: &[KColor]) -> KSeq {
    KSeq { ptr: colors.as_ptr().cast(), len: colors.len() as i64 }
}

fn white() -> KColor {
    rgba(1.0, 1.0, 1.0, 1.0)
}

#[test]
fn headless_texture_lines() {
    let checker = checker_path();
    let shown = checker.clone();
    let (lines, (tex, pair)) = headless_run(move || {
        let (title, path) = (kstr("t"), kstr(&checker));
        let (red, blue) = (rgba(1.0, 0.0, 0.0, 1.0), rgba(0.0, 0.0, 1.0, 1.0));
        let (mut tex, mut pair) = (NO_TEXTURE, NO_TEXTURE);
        // SAFETY: every pointer is to a live local, as compiled code passes them.
        unsafe {
            open_with(&headless(1), 64, 64, &title);
            kestrel_draw_load_texture(&mut tex, &path);
            kestrel_draw_make_texture(&mut pair, 2, 1, &seq(&[red, blue]));
            assert_eq!(kestrel_window_frame(), 1);
            kestrel_draw_texture(&tex, &v2(10.0, 40.0), &white());
            kestrel_draw_texture_rect(&pair, &v2(0.0, 0.0), &v2(32.0, 16.0), &rgba(1.0, 0.5, 0.25, 1.0));
            let part = KRect { pos: v2(0.0, 0.0), size: v2(4.0, 2.0) };
            kestrel_draw_texture_region(&tex, &part, &v2(64.0, 32.0), &v2(-32.0, 16.0), &white());
        }
        (tex, pair)
    });
    assert_eq!((tex.width, tex.height), (4, 4), "the size of the file");
    assert_eq!((pair.width, pair.height), (2, 1));
    assert!(tex.id > 0 && pair.id > 0 && tex.id != pair.id, "{} and {}", tex.id, pair.id);
    let (c, p) = (tex.id, pair.id);
    assert_eq!(
        lines,
        [
            "window.open 64 64 \"t\"".to_string(),
            format!("draw.load_texture \"{shown}\""),
            "draw.make_texture 2 1 2".to_string(),
            format!("draw.texture Texture(id: {c}, width: 4, height: 4) vec2(x: 10, y: 40) Color(r: 1, g: 1, b: 1, a: 1)"),
            format!(
                "draw.texture_rect Texture(id: {p}, width: 2, height: 1) vec2(x: 0, y: 0) vec2(x: 32, y: 16) \
                 Color(r: 1, g: 0.5, b: 0.25, a: 1)"
            ),
            format!(
                "draw.texture_region Texture(id: {c}, width: 4, height: 4) Rect(pos: vec2(x: 0, y: 0), size: vec2(x: 4, y: 2)) \
                 vec2(x: 64, y: 32) vec2(x: -32, y: 16) Color(r: 1, g: 1, b: 1, a: 1)"
            ),
        ]
    );
}

#[test]
fn a_quiet_window_prints_no_texture_lines_but_still_makes_textures() {
    let (lines, tex) = headless_run(|| {
        let title = kstr("t");
        let mut tex = NO_TEXTURE;
        // SAFETY: every pointer is to a live local.
        unsafe {
            open_with(&Settings { quiet: true, ..headless(1) }, 8, 8, &title);
            kestrel_draw_make_texture(&mut tex, 1, 1, &seq(&[white()]));
            assert_eq!(kestrel_window_frame(), 1);
            kestrel_draw_texture(&tex, &v2(0.0, 0.0), &white());
        }
        tex
    });
    assert_eq!(lines, Vec::<String>::new());
    assert_eq!((tex.width, tex.height), (1, 1));
}

#[test]
fn a_texture_made_every_frame_is_freed_every_frame() {
    let (lines, (most, left)) = headless_run(|| {
        let title = kstr("t");
        let (mut tex, mut pair, mut dot) = (NO_TEXTURE, NO_TEXTURE, NO_TEXTURE);
        let (checker, one) = (checker_path(), [white()]);
        let mut most = 0;
        // SAFETY: every pointer is to a live local.
        unsafe {
            open_with(&Settings { quiet: true, ..headless(100) }, 64, 64, &title);
            kestrel_draw_load_texture(&mut tex, &kstr(&checker));
            kestrel_draw_make_texture(&mut pair, 2, 1, &seq(&[white(), white()]));
            while kestrel_window_frame() == 1 {
                // `let dot = draw.make_texture(...)` in the loop body.
                kestrel_draw_make_texture(&mut dot, 1, 1, &seq(&one));
                kestrel_draw_texture(&tex, &v2(10.0, 40.0), &white());
                kestrel_draw_texture_rect(&pair, &v2(0.0, 0.0), &v2(32.0, 16.0), &white());
                kestrel_draw_texture(&dot, &v2(1.0, 1.0), &white());
                most = most.max(gpu::live());
                // The end of the body: compiled code drops `dot`.
                kestrel_runtime::gpu::kestrel_rt_gpu_drop(dot.id);
            }
        }
        (most, gpu::live())
    });
    assert_eq!(lines, Vec::<String>::new());
    assert_eq!(most, 3, "the file's texture, the made one and one a frame: never more");
    assert_eq!(left, 0, "and the window's close freed the rest");
}

#[test]
fn texture_errors_say_what_to_do() {
    let missing = load_texture_in(Mode::Headless, b"missing.png", Path::new("C:/game/missing.png"));
    assert_eq!(missing, Err("draw.load_texture couldn't read missing.png (looked for C:/game/missing.png): no such file".to_string()));

    // A file that exists but isn't an image: this crate's manifest.
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml");
    let not_an_image = load_texture_in(Mode::Headless, b"Cargo.toml", &manifest).unwrap_err();
    let shown = crate::paths::display(&manifest);
    assert_eq!(
        not_an_image,
        format!("draw.load_texture couldn't read Cargo.toml (looked for {shown}): it isn't a PNG, JPG or BMP image")
    );

    let none: [KColor; 0] = [];
    let size_error = |w, h| make_texture_in(Mode::Headless, w, h, &none).unwrap_err();
    assert_eq!(size_error(0, 16), "draw.make_texture needs a width and height above 0, not 0 by 16");
    assert_eq!(size_error(4, -1), "draw.make_texture needs a width and height above 0, not 4 by -1");
    assert_eq!(
        make_texture_in(Mode::Headless, 16, 16, &[white(); 255]).unwrap_err(),
        "draw.make_texture needs width \u{d7} height pixels: 16 by 16 is 256, not 255"
    );
    // The product is worked out in 64 bits, so two big sizes can't wrap round to a matching count.
    assert_eq!(
        size_error(i32::MAX, i32::MAX),
        "draw.make_texture needs width \u{d7} height pixels: 2147483647 by 2147483647 is 4611686014132420609, not 0"
    );
}

#[test]
fn an_empty_texture_names_the_native_that_got_it() {
    assert_eq!(
        lookup("draw.texture_rect", NO_TEXTURE).unwrap_err(),
        "draw.texture_rect got an empty texture; make one with draw.load_texture or draw.make_texture"
    );
    // An id nothing registered (a freed one) is just as empty.
    let freed = KTexture { id: i64::MAX, width: 4, height: 4 };
    assert!(lookup("draw.texture", freed).unwrap_err().starts_with("draw.texture got an empty texture;"));
}

#[test]
fn drawing_uses_the_size_the_gpu_object_has_not_the_field() {
    std::thread::spawn(|| {
        let made = make_texture_in(Mode::Headless, 3, 2, &[white(); 6]).unwrap();
        // A program can assign to a plain field; the object's own size is the one that counts.
        let edited = KTexture { width: 100, height: 100, ..made };
        let found = lookup("draw.texture", edited).unwrap();
        assert_eq!((found.width, found.height), (3, 2));
        assert!(found.texture.is_none(), "a headless texture has no raylib texture");
    })
    .join()
    .unwrap();
}
