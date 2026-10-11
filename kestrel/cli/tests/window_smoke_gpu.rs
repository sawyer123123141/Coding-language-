//! The real-window tests for textures, sprites and (in later tasks) bulk rectangles and 3D:
//! draw on the GPU, take a screenshot and check its pixels, under both `kestrel run` (the JIT)
//! and `kestrel build` (an exe). The shared machinery is in `common/smoke.rs`; the shapes and
//! text are in `window_smoke.rs`.
//!
//! The window is hidden (`KESTREL_HIDDEN=1`), so no window appears while these run. If this
//! machine's driver doesn't render hidden windows, set `KESTREL_SMOKE_VISIBLE=1` in the
//! environment of `cargo test`: then, when the hidden run behaved (exit 0, the expected stdout)
//! and only the screenshot's pixels differ, the program runs again with a visible window and the
//! test asserts on that run. Without it a hidden run whose pixels differ fails at once.

mod common;

use common::smoke::{smoke, Case, How, BLACK, BLUE, GREEN, RED, WHITE};

/// Both ways of making a texture and all three ways of drawing one, in a 64 by 64 window:
///
/// - `checker.png` (red, green, blue and white 2 by 2 cells) stretched over the top-left 32 by 32;
/// - a 2 by 1 texture of red and blue stretched over (32, 0) to (64, 16);
/// - the top row of the checker as a region, drawn with a negative width, so its right-hand
///   colour comes out on the left: green, then red, over (32, 32) to (64, 48);
/// - the whole checker as a region twice as wide as the texture, which repeats it side by side,
///   over (0, 32) to (16, 40);
/// - the checker with a negative height, so it is flipped upside down over (24, 44) to (32, 52);
/// - the checker at its natural size, plain and tinted red;
/// - a texture made in each frame, so the real frees (`UnloadTexture`) run too.
///
/// The PNG sits next to the program: the program loads it by a relative path, which also checks
/// the program-folder rule under both `kestrel run` and the built exe.
const TEXTURES: Case = Case {
    name: "textures",
    program: r#"use window
use draw
use { Rect from draw }

main() {
    window.open(64, 64, "textures")
    let checker = draw.load_texture("checker.png")
    let pair = draw.make_texture(2, 1, [draw.RED, draw.BLUE])
    var n = 0
    while window.frame() {
        n += 1
        draw.clear(draw.BLACK)
        draw.texture_rect(checker, vec2(0, 0), vec2(32, 32), draw.WHITE)
        draw.texture_rect(pair, vec2(32, 0), vec2(32, 16), draw.WHITE)
        let top_row = Rect(pos: vec2(0, 0), size: vec2(4, 2))
        draw.texture_region(checker, top_row, vec2(64, 32), vec2(-32, 16), draw.WHITE)
        // A region twice as wide as the texture repeats it.
        let wide = Rect(pos: vec2(0, 0), size: vec2(8, 4))
        draw.texture_region(checker, wide, vec2(0, 32), vec2(16, 8), draw.WHITE)
        // A negative height covers the rectangle above `pos`, upside down.
        draw.texture_rect(checker, vec2(24, 52), vec2(8, -8), draw.WHITE)
        draw.texture(checker, vec2(4, 52), draw.WHITE)
        draw.texture(checker, vec2(20, 52), draw.RED)
        // A texture made every frame is freed at the next frame, and the last one when the window closes.
        let dot = draw.make_texture(1, 1, [draw.GREEN])
        draw.texture_rect(dot, vec2(36, 56), vec2(8, 4), draw.WHITE)
        if n == 3 {
            window.screenshot("SHOT_PATH")
            window.close()
        }
    }
    print("frames", n)
}
"#,
    stdout: "frames 3\n",
    size: (64, 64),
    pixels: &[
        (8, 8, RED, "the checker's top-left cell, stretched"),
        (24, 8, GREEN, "its top-right cell"),
        (8, 24, BLUE, "its bottom-left cell"),
        (24, 24, WHITE, "its bottom-right cell"),
        (40, 8, RED, "the made texture's left half"),
        (56, 8, BLUE, "the made texture's right half"),
        (40, 40, GREEN, "the mirrored region's left side"),
        (56, 40, RED, "the mirrored region's right side"),
        (48, 24, BLACK, "between the made texture and the mirrored region"),
        (5, 53, RED, "the natural-size checker's top-left cell"),
        (7, 53, GREEN, "its top-right cell"),
        (5, 55, BLUE, "its bottom-left cell"),
        (7, 55, WHITE, "its bottom-right cell"),
        (21, 53, RED, "the red-tinted checker's red cell"),
        (23, 53, BLACK, "its green cell, which a red tint removes"),
        (23, 55, RED, "its white cell, which a red tint turns red"),
        (12, 54, BLACK, "beside the natural-size checker"),
        (2, 34, RED, "the repeating region's first copy, top-left cell"),
        (6, 34, GREEN, "its first copy, top-right cell"),
        (10, 34, RED, "its second copy, which only a repeating texture has: top-left cell"),
        (14, 34, GREEN, "its second copy, top-right cell"),
        (10, 38, BLUE, "its second copy, bottom-left cell"),
        (14, 38, WHITE, "its second copy, bottom-right cell"),
        (26, 46, BLUE, "the upside-down checker's top-left cell, which is the bottom-left one"),
        (30, 46, WHITE, "its top-right cell, which is the bottom-right one"),
        (26, 50, RED, "its bottom-left cell, which is the top-left one"),
        (30, 50, GREEN, "its bottom-right cell, which is the top-right one"),
        (40, 58, GREEN, "the one-pixel texture made in the frame, stretched"),
    ],
    inked: &[],
    assets: &[("assets/checker.png", "checker.png")],
};

/// The same program in a folder whose name has a space and a non-ASCII letter. raylib's own file
/// functions can't open such a path on Windows, so this checks that Rust reads the texture and
/// writes the screenshot.
const TEXTURES_ACCENTED: Case = Case { name: "textures caf\u{e9}", ..TEXTURES };

#[test]
fn textures_and_sprites_draw_under_the_jit() {
    smoke(How::Jit, &TEXTURES);
}

#[test]
fn textures_and_sprites_draw_in_an_exe() {
    smoke(How::Exe, &TEXTURES);
}

#[test]
fn a_folder_with_a_non_ascii_name_works_under_the_jit() {
    smoke(How::Jit, &TEXTURES_ACCENTED);
}

#[test]
fn a_folder_with_a_non_ascii_name_works_in_an_exe() {
    smoke(How::Exe, &TEXTURES_ACCENTED);
}
