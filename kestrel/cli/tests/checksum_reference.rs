//! `checksum(x)` equals the fold the syntax spec defines, computed here in Rust word by word.
use std::fs;
use std::path::Path;
use std::process::Command;

use kestrel_runtime::checksum::{finish, mix, mix_bytes, START};

/// Runs `src` as a program in a fresh folder and returns its output lines.
fn run(name: &str, src: &str) -> Vec<String> {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("checksum_reference_{name}"));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    let file = dir.join("main.kes");
    fs::write(&file, src).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_kestrel")).arg("run").arg(&file).env("KESTREL_POISON", "1").output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).lines().map(str::to_string).collect()
}

/// A value's words, as the spec lists them, mixed into `h`.
#[derive(Clone)]
enum W {
    /// One word: a number, bool, tag or length.
    Word(u64),
    /// A string: its byte length, then its bytes in padded little-endian words.
    Text(&'static str),
}

fn int(n: i64) -> W {
    W::Word(n as u64)
}

fn f32w(x: f32) -> W {
    W::Word(u64::from(x.to_bits()))
}

fn f64w(x: f64) -> W {
    W::Word(x.to_bits())
}

/// `checksum` of a value with these words: the fold from `START`, finished, printed signed.
fn checksum(words: &[W]) -> i64 {
    let h = words.iter().fold(START, |h, w| match w {
        W::Word(x) => mix(h, *x),
        W::Text(s) => mix_bytes(h, s.as_bytes()),
    });
    finish(h) as i64
}

#[test]
fn checksum_matches_the_reference_fold() {
    let src = "enum Shape {
    Dot
    Circle { r: float }
}

struct P {
    a: int
    b: f64
    s: string
    xs: list[int]
}

main() {
    print(checksum(0))
    print(checksum(i64(0)))
    print(checksum(true))
    print(checksum(u8(200)))
    print(checksum(1.5))
    print(checksum(f64(-0.0)))
    print(checksum(vec3(1, 2, 3)))
    print(checksum(\"hello, world!\"))
    let xs: list[int] = [1, 2, 3]
    print(checksum(xs))
    var m = {\"a\": 1, \"b\": 2}
    m.remove(\"a\")
    m[\"c\"] = 3
    print(checksum(m))
    let n: int? = none
    print(checksum(n))
    let s: int? = 4
    print(checksum(s))
    print(checksum(Shape.Circle(r: 2.5)))
    print(checksum(P(a: -7, b: 0.25, s: \"hi\", xs: [4, 5])))
    print(checksum(vec4(1, 2, 3, 4)))
    print(checksum(ivec4(-1, 2, 3, 4)))
    let z = 0.0
    print(checksum(z / z))
    print(checksum(f64(z / z)))
    let fs: [f64] = [0.5, -2.0]
    print(checksum(fs))
    print(checksum(\"\"))
    print(checksum(-3))
}
";
    let lines = run("fold", src);
    let want: Vec<Vec<W>> = vec![
        vec![int(0)],
        vec![int(0)],
        vec![int(1)],
        vec![int(200)],
        vec![f32w(1.5)],
        vec![f64w(-0.0)],
        vec![f32w(1.0), f32w(2.0), f32w(3.0)],
        vec![W::Text("hello, world!")],
        vec![int(3), int(1), int(2), int(3)],
        // The live count, then each live key and value in insertion order.
        vec![int(2), W::Text("b"), int(2), W::Text("c"), int(3)],
        vec![int(0)],
        vec![int(1), int(4)],
        // The variant number, then its fields.
        vec![int(1), f32w(2.5)],
        vec![int(-7), f64w(0.25), W::Text("hi"), int(2), int(4), int(5)],
        vec![f32w(1.0), f32w(2.0), f32w(3.0), f32w(4.0)],
        vec![int(-1), int(2), int(3), int(4)],
        // Every NaN is the canonical quiet NaN.
        vec![W::Word(0x7FC0_0000)],
        vec![W::Word(0x7FF8_0000_0000_0000)],
        vec![int(2), f64w(0.5), f64w(-2.0)],
        vec![W::Text("")],
        vec![int(-3)],
    ];
    let want: Vec<String> = want.iter().map(|w| checksum(w).to_string()).collect();
    assert_eq!(lines, want);
}
