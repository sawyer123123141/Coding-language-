//! The modules that ship inside the compiler: `window`, `draw`, `draw3d` and `input`.

use std::fs;
use std::path::{Path, PathBuf};

use kestrel_driver::{compile, Compiled, DiagStyle};

/// A fresh directory holding `files` (name, text).
fn project(name: &str, files: &[(&str, &str)]) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("driver_{name}"));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    for (file, text) in files {
        fs::write(dir.join(file), text).unwrap();
    }
    dir
}

/// Compiles `src` as `main.kes` in `dir`, with short diagnostics.
fn build(dir: &Path, src: &str) -> (Option<Compiled>, String) {
    compile(&dir.join("main.kes").to_string_lossy(), src, DiagStyle::Short)
}

const PROGRAM: &str = "use draw
use { Color, RED from draw }
use { Key from input }
let BG = Color(0.2, 0.2, 0.2)
let UP = vec2(0, 1)
main() {
    print(RED, BG)
    print(Key.Space, UP)
}
";

#[test]
fn the_built_in_modules_load_in_the_order_they_are_used() {
    let dir = project("order", &[]);
    let (compiled, text) = build(&dir, PROGRAM);
    assert_eq!(text, "");
    assert_eq!(compiled.expect("compiles").builtins, ["draw", "input"]);

    let src = "use window\nuse input\nuse draw\nuse { Color from draw }\nmain() {}\n";
    let (compiled, text) = build(&dir, src);
    assert_eq!(text, "");
    assert_eq!(compiled.expect("compiles").builtins, ["window", "input", "draw"]);

    let (compiled, _) = build(&dir, "main() {}\n");
    assert!(compiled.expect("compiles").builtins.is_empty());
}

#[test]
fn draw3d_is_a_built_in_module_that_brings_draw_with_it() {
    let dir = project("draw3d", &[]);
    // It takes its `Color` from `draw`, so `draw` is loaded even if the program never says `use draw`.
    let src = "use draw3d\nuse { Camera from draw3d }\nmain() {\n    print(Camera(pos: vec3(0, 0, -3), target: vec3(0, 0, 0)))\n}\n";
    let (compiled, text) = build(&dir, src);
    assert_eq!(text, "");
    assert_eq!(compiled.expect("compiles").builtins, ["draw3d", "draw"]);

    let (compiled, text) = build(&dir, "use window\nuse draw\nuse draw3d\nmain() {}\n");
    assert_eq!(text, "");
    assert_eq!(compiled.expect("compiles").builtins, ["window", "draw", "draw3d"]);
}

#[test]
fn a_module_can_use_a_built_in_module() {
    let dir = project("nested", &[("shapes.kes", "use { Color, RED from draw }\npub tint() -> Color { RED }\n")]);
    let (compiled, text) = build(&dir, "use shapes\nuse draw\nmain() {\n    print(shapes.tint())\n}\n");
    assert_eq!(text, "");
    assert_eq!(compiled.expect("compiles").builtins, ["draw"]);
}

#[test]
fn built_in_files_have_virtual_paths_and_never_change() {
    let dir = project("virtual", &[]);
    let (compiled, _) = build(&dir, PROGRAM);
    let files = compiled.expect("compiles").files;
    let paths: Vec<String> = files.iter().map(|f| f.path.to_string_lossy().replace('\\', "/")).collect();
    assert_eq!(paths[1..], ["<builtin>/draw.kes", "<builtin>/input.kes"]);
    assert!(files[1..].iter().all(|f| f.modified.is_none() && !f.changed()), "{files:?}");
}

#[test]
fn a_file_named_after_a_built_in_module_is_refused() {
    let message = |name: &str| format!("`{name}` is a built-in module, so {name}.kes can't be used as a module; rename the file");
    for name in ["window", "draw", "draw3d", "input"] {
        let dir = project(&format!("shadow_{name}"), &[(&format!("{name}.kes"), "pub f() -> int { 1 }\n")]);
        let (compiled, text) = build(&dir, &format!("use {name}\nmain() {{}}\n"));
        assert!(compiled.is_none());
        assert_eq!(text, format!("1:5: error: {}\n", message(name)));
    }
    // Only a `use` of the name is refused: the file beside the program may sit unused.
    let dir = project("shadow_unused", &[("draw.kes", "pub f() -> int { 1 }\n")]);
    let (compiled, text) = build(&dir, "use input\nmain() {}\n");
    assert_eq!(text, "");
    assert!(compiled.is_some());
    // The error is on the clause that names the module.
    let dir = project("shadow_clause", &[("window.kes", "")]);
    let (_, text) = build(&dir, "use input\nuse { Key from input }\nuse window\nmain() {}\n");
    assert_eq!(text, format!("3:5: error: {}\n", message("window")));
}

#[test]
fn a_module_file_beside_a_module_that_uses_a_built_in_is_refused_too() {
    let dir = project("shadow_nested", &[("shapes.kes", "use draw\n"), ("draw.kes", "pub f() -> int { 1 }\n")]);
    let (compiled, text) = build(&dir, "use shapes\nmain() {}\n");
    assert!(compiled.is_none());
    assert!(text.contains("`draw` is a built-in module, so draw.kes can't be used as a module; rename the file"), "{text}");
}

#[test]
fn only_built_in_modules_declare_natives() {
    let message = "only Kestrel's built-in modules can declare native functions";
    let dir = project("natives", &[("geom.kes", "pub pure area(x: int) -> int { x }\n\npub native g(x: int)\n")]);

    let (compiled, text) = build(&dir, "native f()\nmain() {}\n");
    assert!(compiled.is_none());
    assert_eq!(text, format!("1:8: error: {message}\n"));

    let (compiled, text) = build(&dir, "main() {}\npub native f()\n\nnative g(x: int) -> bool\n");
    assert!(compiled.is_none());
    assert_eq!(text, format!("2:12: error: {message}\n4:8: error: {message}\n"));

    // In a module file, the error names that file.
    let (compiled, text) = compile(&dir.join("main.kes").to_string_lossy(), "use geom\nmain() {}\n", DiagStyle::Full);
    assert!(compiled.is_none());
    assert!(text.contains(&format!("error: {message}")) && text.contains("geom.kes:3:12"), "{text}");
}

#[test]
fn only_built_in_modules_name_gpu_handle() {
    let message = "`GpuHandle` is only for Kestrel's built-in modules";
    let dir = project("gpu_handle", &[("geom.kes", "pub struct Holder {\n    h: GpuHandle\n}\n")]);

    // The program and a module file are refused, even with built-in modules loaded beside them.
    let (compiled, text) = build(&dir, "use draw\nstruct Mine {\n    h: GpuHandle\n}\nmain() {}\n");
    assert!(compiled.is_none());
    assert_eq!(text, format!("3:8: error: {message}\n"));

    let (compiled, text) = build(&dir, "use draw\nuse geom\nmain() {}\n");
    assert!(compiled.is_none());
    assert_eq!(text, format!("2:8: error: {message}\n"));
}

#[test]
fn the_built_in_modules_themselves_compile_clean() {
    let dir = project("clean", &[]);
    let src = "use window\nuse draw\nuse draw3d\nuse input\nmain() {}\n";
    let (compiled, text) = build(&dir, src);
    assert_eq!(text, "");
    let module = compiled.expect("compiles").module;
    let natives: Vec<&str> = module.funcs.iter().filter_map(|f| f.native.as_deref()).collect();
    assert_eq!(natives.len(), 9 + 15 + 4 + 5, "{natives:?}");
    assert!(natives.contains(&"kestrel_window_open") && natives.contains(&"kestrel_draw_text_width") && natives.contains(&"kestrel_input_wheel"));
    assert!(natives.contains(&"kestrel_draw_load_texture") && natives.contains(&"kestrel_draw_texture_region"));
    assert!(natives.contains(&"kestrel_draw_rects"));
    assert!(natives.contains(&"kestrel_draw3d_camera") && natives.contains(&"kestrel_draw3d_line"));
}

#[test]
fn a_native_parameter_may_share_its_functions_name_but_a_local_may_not() {
    // `draw.text(text: string, ...)`: a native's parameters are labels.
    let dir = project("native_labels", &[]);
    let (_, text) = build(&dir, "use draw\nmain() {\n    draw.text(text: \"x\", pos: vec2(0, 0), size: 1, color: draw.RED)\n}\n");
    assert_eq!(text, "");

    // A function with a body keeps the rule: `text` would hide the imported function.
    let dir = project("local_names", &[("geom.kes", "pub text(text: string) -> string {\n    text\n}\n")]);
    let (compiled, text) = build(&dir, "use geom\nmain() {}\n");
    assert!(compiled.is_none());
    assert_eq!(text, "1:10: error: `text` is already defined: it's imported; pick another name\n");
}

extern "C" fn never_called() {}

/// The program from the plan's `tests/programs/builtin_modules.kes`, run in the JIT with a
/// do-nothing address for every native (it never calls one). It joins `tests/programs` once the
/// CLI links the window library, because until then the JIT and the linker both refuse natives
/// that have no implementation.
#[test]
fn a_program_using_built_in_modules_prints_their_structs_and_enums() {
    let dir = project("run", &[]);
    let (compiled, text) = build(&dir, PROGRAM);
    assert_eq!(text, "");
    let module = compiled.expect("compiles").module;
    let natives: Vec<(&str, *const u8)> =
        module.funcs.iter().filter_map(|f| f.native.as_deref()).map(|name| (name, never_called as *const u8)).collect();
    let program = kestrel_codegen::JitProgram::compile_with(&module, kestrel_codegen::Options::default(), &natives).unwrap();

    // Run `main` here rather than with `run_main`, which uses a thread whose output can't be captured.
    let main: extern "C" fn() = unsafe { std::mem::transmute(program.fn_ptr("main").expect("main")) };
    kestrel_runtime::print::capture_output();
    main();
    let out = String::from_utf8(kestrel_runtime::print::take_output()).unwrap();
    assert_eq!(out, "Color(r: 1, g: 0, b: 0, a: 1) Color(r: 0.2, g: 0.2, b: 0.2, a: 1)\nKey.Space vec2(x: 0, y: 1)\n");
}
