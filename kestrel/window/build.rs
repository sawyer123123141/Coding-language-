//! Compiles the vendored raylib (`raylib/`) into a static library.
//!
//! There is no bindgen: the functions Kestrel uses are declared by hand in
//! `src/raylib.rs`. Audio and 3D models are switched off.

fn main() {
    println!("cargo:rerun-if-changed=raylib");
    println!("cargo:rerun-if-changed=build.rs");

    cc::Build::new()
        .files(
            ["rcore.c", "rshapes.c", "rtextures.c", "rtext.c", "rglfw.c"]
                .map(|file| format!("raylib/{file}")),
        )
        .include("raylib")
        .include("raylib/external/glfw/include")
        .define("PLATFORM_DESKTOP_GLFW", None)
        .define("GRAPHICS_API_OPENGL_33", None)
        .define("SUPPORT_MODULE_RAUDIO", "0")
        .define("SUPPORT_MODULE_RMODELS", "0")
        .warnings(false)
        .compile("raylib");

    // The build script runs on the host, so ask for the target's OS.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        for lib in ["opengl32", "gdi32", "winmm", "user32", "shell32"] {
            println!("cargo:rustc-link-lib=dylib={lib}");
        }
    }
}
