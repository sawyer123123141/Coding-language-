//! `kestrel check --report` on loops that hoisting changed: the text is what it was before the
//! pass.
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// A fresh directory holding `files` (name, text).
fn project(name: &str, files: &[(&str, &str)]) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("report_hoist_{name}"));
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

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

#[test]
fn math_moved_out_of_a_loop_reads_as_written() {
    // Hoisting moves `sin(k) * 4.0`, `x % z` and `-exp(k)` out of the loops; the text is what it
    // was before the pass (recorded with it turned off).
    let src = "pub pick(xs: list[int], k: float, n: int) -> int {
    var s = 0
    for i from 0 to n {
        s += xs[int(sin(k) * 4.0)]
    }
    s
}

pub rem(xs: list[int], x: float, z: float, n: int) -> int {
    var s = 0
    for i from 0 to n {
        s += xs[int(x % z)]
    }
    s
}

pub mix(xs: list[int], k: float, n: int) -> int {
    var s = 0
    for i from 0 to n {
        s += xs[int(-exp(k) + sin(k) * 4.0 * float(i))]
    }
    s
}

main() {}
";
    let dir = project("hoisted", &[("hoisted.kes", src)]);
    let out = kestrel(&["check", "--report"], &dir.join("hoisted.kes"));
    assert!(out.status.success(), "{}", stderr(&out));
    let text = stdout(&out);
    for line in [
        "      line 4: bounds check: `xs[int((...) * 4)]` may be outside `xs` (`int((...) * 4)` can be any `int`)\n",
        "      line 4: overflow check: `s + xs[int((...) * 4)]` may exceed `int` (`s` can be any `int`; `xs[int((...) * 4)]` can be any `int`)\n",
        "      line 12: bounds check: `xs[int(x % z)]` may be outside `xs` (`int(x % z)` can be any `int`)\n",
        "      line 12: overflow check: `s + xs[int(x % z)]` may exceed `int` (`s` can be any `int`; `xs[int(x % z)]` can be any `int`)\n",
        "      line 20: bounds check: `xs[int(-(...) + (((...) * 4) * float(i))...` may be outside `xs` (`int(-(...) + (((...) * 4) * float(i)))` can be any `int`)\n",
        "      line 20: overflow check: `s + xs[int(-(...) + (((...) * 4) * float...` may exceed `int` (`s` can be any `int`; `xs[int(-(...) + (((...) * 4) * float(i))...` can be any `int`)\n",
    ] {
        assert!(text.contains(line), "missing {line}in {text}");
    }
}
