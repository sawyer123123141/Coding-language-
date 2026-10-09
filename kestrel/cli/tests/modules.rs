//! Programs split across files with `use`.
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// A fresh directory holding `files` (name, text).
fn project(name: &str, files: &[(&str, &str)]) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    for (file, text) in files {
        fs::write(dir.join(file), text).unwrap();
    }
    dir
}

fn kestrel(args: &[&str], file: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_kestrel")).args(args).arg(file).output().unwrap()
}

const GEOM: &str = "pub struct Point {
    x: int
    y: int

    pure sum(self) -> int { self.x + self.y }
}

pub let ORIGIN_X = 10

pub pure dist2(a: Point, b: Point) -> int {
    let dx = a.x - b.x
    let dy = a.y - b.y
    dx * dx + dy * dy
}

helper() -> int { 1 }

pub enum Shape {
    Dot
    Box { w: int, h: int }
}

pub pure at(xs: [int], i: int) -> int {
    xs[i]
}
";

#[test]
fn imported_names_and_qualified_access() {
    let main = "use { Point, dist2 from geom }
use geom

main() {
    let a = Point(1, 2)
    let b: Point = Point(4, 6)
    print(dist2(a, b), a.sum(), geom.ORIGIN_X)
    let s = geom.Shape.Box(w: 2, h: 3)
    print(s, geom.dist2(b, a), a)
}
";
    let dir = project("modules_ok", &[("main.kes", main), ("geom.kes", GEOM)]);
    let out = kestrel(&["run"], &dir.join("main.kes"));
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    assert_eq!(String::from_utf8_lossy(&out.stdout), "25 3 10\nShape.Box(w: 2, h: 3) 25 Point(x: 1, y: 2)\n");
}

#[test]
fn module_errors_point_at_the_right_file() {
    let main = "use { helper, nope from geom }
use missing
use geom

main() {
    let dist2 = 1
    print(geom)
}
";
    let dir = project("modules_errors", &[("main.kes", main), ("geom.kes", GEOM)]);
    let out = kestrel(&["check", "--diagnostics=short"], &dir.join("main.kes"));
    let err = String::from_utf8_lossy(&out.stderr);
    for expected in [
        "1:7: error: `helper` isn't `pub` in `geom`",
        "1:15: error: `geom` has no `nope`",
        "2:5: error: can't find module `missing`: there's no `missing.kes` next to this file",
        "7:11: error: `geom` is a module; use one of its items, like `geom.something`",
    ] {
        assert!(err.contains(expected), "missing {expected:?} in:\n{err}");
    }
    // Errors inside an imported file name that file.
    let bad = project("modules_bad_import", &[("main.kes", "use geom\nmain() {}\n"), ("geom.kes", "pub f() -> int { true }\n")]);
    let out = kestrel(&["check"], &bad.join("main.kes"));
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("geom.kes:1:18"), "{err}");
}

#[test]
fn runtime_errors_report_the_line_in_their_own_file() {
    let main = "use { at from geom }\nmain() {\n    print(at([1, 2], 5))\n}\n";
    let dir = project("modules_runtime", &[("main.kes", main), ("geom.kes", GEOM)]);
    let out = kestrel(&["run"], &dir.join("main.kes"));
    assert_eq!(out.status.code(), Some(101));
    // `xs[i]` is on line 24 of geom.kes.
    assert!(String::from_utf8_lossy(&out.stderr).contains("index 5 out of bounds for length 2 on line 24"));
}
