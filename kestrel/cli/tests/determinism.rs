//! The determinism promise across builds: the JIT, `run --release`, a checked exe and a
//! `--release` exe print the same thing for the `det_*` programs, and the number of threads a
//! `par for` uses changes nothing. `det_math.kes` pins the JIT's copy of `libm` (built with its
//! `arch` feature on through `cranelift-codegen`) against the exes' copy (off), bit for bit.
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

mod common;
use common::directives;
use common::hermetic;

use kestrel_runtime::checksum::{finish, mix, START};
use kestrel_runtime::math;
use kestrel_runtime::panic::kestrel_rt_fmod_f32;

const BUILDS: [&str; 4] = ["jit", "jit --release", "exe", "exe --release"];

fn program(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../tests/programs").join(format!("{name}.kes"))
}

/// The program's `// expect:` lines, one per line.
fn expected(name: &str) -> String {
    let src = fs::read_to_string(program(name)).unwrap();
    directives::parse(name, &src).stdout
}

fn kestrel() -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_kestrel"));
    hermetic(&mut cmd);
    cmd
}

/// Runs `cmd` and returns its standard output, which must be there and the run must succeed.
fn stdout_of(mut cmd: Command, what: &str) -> String {
    let out = cmd.output().unwrap();
    assert!(out.status.success(), "{what}: {}", String::from_utf8_lossy(&out.stderr));
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    assert!(!text.trim().is_empty(), "{what}: printed nothing");
    text
}

/// `kestrel run` of the program, checked or `--release`, with `KESTREL_THREADS` when given.
fn run_jit(name: &str, release: bool, threads: Option<&str>) -> String {
    let mut cmd = kestrel();
    cmd.arg("run");
    if release {
        cmd.arg("--release");
    }
    cmd.arg(program(name));
    if let Some(t) = threads {
        cmd.env("KESTREL_THREADS", t);
    }
    stdout_of(cmd, &format!("{name}: run{}", if release { " --release" } else { "" }))
}

/// Builds the program into its own folder under `CARGO_TARGET_TMPDIR` and returns the exe's path.
fn build_exe(name: &str, release: bool) -> PathBuf {
    let tag = if release { "release" } else { "checked" };
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("determinism_{name}_{tag}"));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    let exe = dir.join(format!("{name}{}", std::env::consts::EXE_SUFFIX));
    let mut cmd = kestrel();
    cmd.arg("build").arg(program(name)).arg("-o").arg(&exe);
    if release {
        cmd.arg("--release");
    }
    let out = cmd.output().unwrap();
    assert!(out.status.success(), "{name}: build {tag}: {}", String::from_utf8_lossy(&out.stderr));
    exe
}

fn run_exe(exe: &Path, what: &str, threads: Option<&str>) -> String {
    let mut cmd = Command::new(exe);
    hermetic(&mut cmd);
    if let Some(t) = threads {
        cmd.env("KESTREL_THREADS", t);
    }
    stdout_of(cmd, what)
}

/// The program's standard output in the four builds, in `BUILDS` order.
fn four_builds(name: &str) -> Vec<String> {
    vec![
        run_jit(name, false, None),
        run_jit(name, true, None),
        run_exe(&build_exe(name, false), &format!("{name}: checked exe"), None),
        run_exe(&build_exe(name, true), &format!("{name}: release exe"), None),
    ]
}

/// The first line where `a` and `b` differ, for a failure message.
fn first_difference(a: &str, b: &str) -> String {
    for (i, (x, y)) in a.lines().zip(b.lines()).enumerate() {
        if x != y {
            return format!("line {}: `{x}` against `{y}`", i + 1);
        }
    }
    format!("{} lines against {} lines", a.lines().count(), b.lines().count())
}

/// All four outputs are equal to the first and to the program's `// expect:` lines.
fn assert_same_everywhere(name: &str, outputs: &[String]) {
    let want = expected(name);
    for (build, out) in BUILDS.iter().zip(outputs) {
        assert!(out.trim() == want.trim(), "{name}: `{build}` differs from the expected lines, first at {}", first_difference(out, &want));
        assert!(out == &outputs[0], "{name}: `{build}` differs from the JIT, first at {}", first_difference(out, &outputs[0]));
    }
}

#[test]
fn the_jit_and_both_exes_print_the_same() {
    for name in ["det_rng_checksum", "det_sim", "det_snapshot", "det_par", "det_math_float"] {
        assert_same_everywhere(name, &four_builds(name));
    }
}

#[test]
fn the_jit_and_exe_math_agree_bit_for_bit() {
    assert_same_everywhere("det_math", &four_builds("det_math"));
}

/// The checksum of a `list[float]` the way compiled code folds it: the length, then each item's 32
/// bits zero-extended, every NaN as the canonical quiet NaN.
fn list_checksum(items: &[f32]) -> i64 {
    let mut h = mix(START, items.len() as u64);
    for &x in items {
        let bits = if x.is_nan() { 0x7FC0_0000 } else { x.to_bits() };
        h = mix(h, u64::from(bits));
    }
    finish(h) as i64
}

/// A million inputs through each `float` function: the runtime's `kestrel_rt_*f` functions, called
/// from Rust, fold to the checksums the compiled program prints, in the JIT and in the program's
/// `// expect:` lines. The inputs are dyadic, so Rust computes the same `f32`s the program does.
#[test]
fn the_float_sweep_matches_the_runtime() {
    const N: usize = 1_000_000;
    let mut cols: Vec<Vec<f32>> = (0..13).map(|_| Vec::with_capacity(N)).collect();
    for i in 0..N as i64 {
        let a = (i - 500_000) as f32 * 0.0009765625f32;
        let big = (i - 500_000) as f32 * 1024.0f32;
        let e = (i - 500_000) as f32 * 0.0001953125f32;
        let p = (i + 1) as f32 * 0.0009765625f32;
        let y = (i % 2001 - 1000) as f32 * 0.03125f32;
        let w = (i % 1999 - 999) as f32 * 0.25f32;
        // 0.0000019073486328125 is 2^-19: the division by 2^19 gives the same bits as the program's product.
        let u = (i - 500_000) as f32 / 524_288.0f32;
        let d = (i % 97 + 1) as f32 * 0.0625f32;
        let row = [
            math::kestrel_rt_sinf(a),
            math::kestrel_rt_cosf(a),
            math::kestrel_rt_tanf(a),
            math::kestrel_rt_sinf(big),
            math::kestrel_rt_cosf(big),
            math::kestrel_rt_expf(e),
            math::kestrel_rt_logf(p),
            math::kestrel_rt_powf(p, y),
            math::kestrel_rt_atan2f(a, w),
            math::kestrel_rt_atanf(a),
            math::kestrel_rt_asinf(u),
            math::kestrel_rt_acosf(u),
            kestrel_rt_fmod_f32(a, d),
        ];
        for (col, x) in cols.iter_mut().zip(row) {
            col.push(x);
        }
    }
    let want: Vec<String> = cols.iter().map(|c| list_checksum(c).to_string()).collect();
    println!("computed in Rust:\n{}", want.iter().map(|w| format!("// expect: {w}")).collect::<Vec<_>>().join("\n"));
    let expected = expected("det_math_float");
    assert_eq!(expected.lines().collect::<Vec<_>>(), want, "the program's `// expect:` lines against the checksums computed in Rust");
    let jit = run_jit("det_math_float", false, None);
    assert_eq!(jit.lines().collect::<Vec<_>>(), want, "the JIT's output against the checksums computed in Rust");
}

#[test]
fn the_thread_count_changes_nothing() {
    for name in ["det_par", "det_sim"] {
        let want = expected(name);
        let exe = build_exe(name, true);
        for threads in ["1", "2", "16"] {
            let jit = run_jit(name, false, Some(threads));
            assert!(jit.trim() == want.trim(), "{name}: JIT with {threads} threads, first difference at {}", first_difference(&jit, &want));
            let out = run_exe(&exe, &format!("{name}: exe with {threads} threads"), Some(threads));
            assert!(out.trim() == want.trim(), "{name}: exe with {threads} threads, first difference at {}", first_difference(&out, &want));
        }
    }
}
