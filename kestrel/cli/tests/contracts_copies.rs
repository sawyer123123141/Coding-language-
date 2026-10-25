//! How contract errors name copies of generics: a loop in a closure of a copy, or in a field
//! default's closure of an instance, names what it was lowered for; a copy that made itself says
//! where; a library's copy is written without its module; the errors come in source order.
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// A fresh directory holding `files` (name, text).
fn project(name: &str, files: &[(&str, &str)]) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("contracts_copies_{name}"));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    for (file, text) in files {
        fs::write(dir.join(file), text).unwrap();
    }
    dir
}

fn kestrel(args: &[&str], file: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_kestrel")).args(args).arg(file).output().expect("run kestrel")
}

fn check_short(name: &str, src: &str) -> (Option<i32>, String) {
    let dir = project(name, &[(&format!("{name}.kes"), src)]);
    let out = kestrel(&["check", "--diagnostics=short"], &dir.join(format!("{name}.kes")));
    (out.status.code(), String::from_utf8_lossy(&out.stderr).into_owned())
}

const CLOSURE_LOOPS: &str = "struct Box[T] {
    f: pure (list[T]) -> int = (xs) {
        var n = 0
        @prove(no_alloc)
        for x in xs {
            let l: list[int] = [1]
            n += l.len
        }
        n
    }
}

pure keeps[T](xs: list[T]) -> int {
    let g = (ys: list[T]) {
        var n = 0
        @prove(no_alloc)
        for y in ys {
            let l: list[int] = [1]
            n += l.len
        }
        n
    }
    g(xs)
}

main() {
    let a = Box[int]()
    let b = Box[string]()
    print(a.f([1]), b.f([\"s\"]), keeps([1]), keeps([\"s\"]))
}
";

#[test]
fn a_loop_in_a_closure_names_its_copy() {
    // The loop's code moves to the closure's, which has no copy note of its own: the site keeps
    // what it was lowered for. Nothing names a line for a field default's instance.
    let (code, text) = check_short("closure_loops", CLOSURE_LOOPS);
    assert_eq!(code, Some(1), "{text}");
    let new_list = "can't be proven `no_alloc`: it allocates on line 6: `[...]` makes a new list";
    let in_keeps = "can't be proven `no_alloc`: it allocates on line 18: `[...]` makes a new list";
    let want = format!(
        "4:16: error: the loop at line 5 (made by `Box[int]`) {new_list}
4:16: error: the loop at line 5 (made by `Box[string]`) {new_list}
16:16: error: the loop at line 17 (made by `keeps[int]` on line 29) {in_keeps}
16:16: error: the loop at line 17 (made by `keeps[string]` on line 29) {in_keeps}
"
    );
    assert_eq!(text, want);
}

#[test]
fn copy_errors_come_in_source_order_and_a_function_copy_names_itself_once() {
    // `Stack[int].push`'s copy is made after `first[string]`'s but is reported second, in source
    // order; `first[string]` made itself, so it says only where.
    let src = "@prove(no_alloc)
pure first[T](xs: list[T]) -> T {
    xs[0]
}

pure unused[T](x: T) -> T { x }

struct Stack[T] {
    items: list[T] = []

    @prove(no_alloc)
    push(var self, v: T) { self.items.add(v) }
}

main() {
    let a: list[int] = [1, 2]
    let b: list[string] = [\"p\"]
    var s = Stack[int]()
    s.push(4)
    print(first(a), first(b), s.items)
}
";
    let (code, text) = check_short("order", src);
    assert_eq!(code, Some(1), "{text}");
    let want = "1:8: error: `first[string]` (made on line 20) can't be proven `no_alloc`: it allocates on line 3: makes a copy of `xs[0]`
11:12: error: `Stack[int].push` (made by `Stack[int]` on line 19) can't be proven `no_alloc`: it allocates on line 12: `add` may grow `self.items`
";
    assert_eq!(text, want);
}

const GEOM: &str = "pub struct Item {
    name: string
}

pub struct Stack[T] {
    items: list[T] = []

    @prove(no_alloc)
    pub put(var self, v: T) { self.items.add(v) }
}
";

const MAIN: &str = "use { Stack, Item from geom }

struct Mine {
    n: int
}

main() {
    var s = Stack[Item]()
    s.put(Item(name: \"a\"))
    var t = Stack[Mine]()
    t.put(Mine(n: 1))
    print(s, t)
}
";

#[test]
fn a_library_copy_is_written_without_its_module() {
    // `geom.Stack[geom.Item].put` is `Stack[Item].put` in `geom.kes`, in errors and in the report.
    let dir = project("library_names", &[("geom.kes", GEOM), ("main.kes", MAIN)]);
    let out = kestrel(&["check", "--diagnostics=short"], &dir.join("main.kes"));
    let text = String::from_utf8_lossy(&out.stderr).into_owned();
    assert_eq!(out.status.code(), Some(1), "{text}");
    let want = "8:12: error: `Stack[Item].put` (made by `Stack[Item]` on main.kes:9) can't be proven `no_alloc`: it allocates on line 9: makes a copy of the string `name`
8:12: error: `Stack[Mine].put` (made by `Stack[Mine]` on main.kes:11) can't be proven `no_alloc`: it allocates on line 9: `add` may grow `self.items`
";
    assert_eq!(text, want);
    let out = kestrel(&["check", "--report"], &dir.join("main.kes"));
    let report = String::from_utf8_lossy(&out.stdout).into_owned() + &String::from_utf8_lossy(&out.stderr);
    assert!(report.contains("geom.kes\n  function `Stack[Item].put` (line 9): its own copy is never called"), "{report}");
    assert!(!report.contains("geom.Item"), "{report}");
}
