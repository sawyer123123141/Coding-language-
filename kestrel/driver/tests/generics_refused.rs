//! Temporary (Task 6b deletes it): the driver gives no module for a program with a generic
//! declaration, whichever entry point it goes through.

use std::fs;
use std::path::Path;

use kestrel_driver::DiagStyle;

const SRC: &str = "struct Box[T] {\n    v: T\n}\nmain() { print(1) }\n";

fn entry() -> String {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("driver_generics_refused");
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    let path = dir.join("main.kes");
    fs::write(&path, SRC).unwrap();
    path.to_string_lossy().into_owned()
}

#[test]
fn compile_unoptimized_gives_no_module() {
    let (compiled, text) = kestrel_driver::compile_unoptimized(&entry(), SRC, DiagStyle::Short, true);
    assert!(compiled.is_none());
    assert!(text.contains("generics aren't supported yet"), "{text}");
}

#[test]
fn compile_gives_no_module() {
    let (compiled, text) = kestrel_driver::compile(&entry(), SRC, DiagStyle::Short);
    assert!(compiled.is_none());
    assert!(text.contains("generics aren't supported yet"), "{text}");
}
