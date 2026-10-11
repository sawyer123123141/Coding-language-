//! The `draw` texture natives, called the way compiled code calls them, in headless mode.

use std::path::Path;

use super::{headless, headless_run, kstr, rgba, v2};
use crate::abi::{KColor, KRect, KSeq, KTexture};
use crate::gpu;
use crate::natives_texture::*;
use crate::natives_window::*;
use crate::state::{Mode, Settings};

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
fn a_texture_loads_from_a_folder_with_a_non_ascii_name() {
    // raylib's own file functions can't open such a path on Windows; Rust reads the file instead.
    let dir = std::env::temp_dir().join(format!("kestrel_caf\u{e9}_texture_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("checker.png");
    std::fs::copy(checker_path(), &path).unwrap();
    let loaded = load_texture_in(Mode::Headless, b"checker.png", &path);
    std::fs::remove_dir_all(&dir).unwrap();
    let texture = loaded.expect("the checker loads from a folder called caf\u{e9}");
    assert_eq!((texture.width, texture.height), (4, 4));
}

#[test]
fn a_file_that_cant_be_read_gives_the_systems_reason_not_a_wrong_guess() {
    // A folder can't be read as a file.
    let folder = Path::new(env!("CARGO_MANIFEST_DIR"));
    let message = load_texture_in(Mode::Headless, b"window", folder).unwrap_err();
    let start = format!("draw.load_texture couldn't read window (looked for {}): ", crate::paths::display(folder));
    let reason = message.strip_prefix(&start).unwrap_or_else(|| panic!("{message}"));
    assert!(!reason.is_empty() && !reason.contains("os error") && reason != "no such file", "{message}");
    assert!(!reason.contains("PNG"), "a folder isn't called a bad image: {message}");
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
