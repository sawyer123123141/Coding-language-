//! `Rng` from the built-in module `random` equals SplitMix64, computed here in Rust.
use std::fs;
use std::path::Path;
use std::process::Command;

/// One step of SplitMix64: the new state and the output.
fn splitmix(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// Runs `src` as a program in a fresh folder and returns its output lines.
fn run(name: &str, src: &str) -> Vec<String> {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("rng_reference_{name}"));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    let file = dir.join("main.kes");
    fs::write(&file, src).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_kestrel")).arg("run").arg(&file).env("KESTREL_POISON", "1").output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).lines().map(str::to_string).collect()
}

#[test]
fn next_matches_splitmix64() {
    let src = "use { Rng from random }
main() {
    for seed in [42, -1, 0] {
        var r = Rng(seed: seed)
        for i from 0 to 8 {
            print(r.next())
        }
    }
}
";
    let lines = run("next", src);
    let mut want = Vec::new();
    for seed in [42i64, -1, 0] {
        let mut state = seed as u64;
        for _ in 0..8 {
            want.push((splitmix(&mut state) as i64).to_string());
        }
    }
    assert_eq!(lines, want);
}

#[test]
fn randf_matches_the_reference() {
    let src = "use { Rng from random }
main() {
    var r = Rng(seed: 7)
    for i from 0 to 8 {
        print(r.randf())
    }
}
";
    let lines = run("randf", src);
    let mut state = 7u64;
    for (i, line) in lines.iter().enumerate() {
        let draw = splitmix(&mut state);
        let mut f = ((draw >> 11) as f64 * (1.0 / 9_007_199_254_740_992.0)) as f32;
        if f >= 1.0 {
            f = f32::from_bits(1.0f32.to_bits() - 1);
        }
        let got: f32 = line.parse().unwrap_or_else(|_| panic!("line {i}: {line}"));
        assert_eq!(got.to_bits(), f.to_bits(), "draw {i}: printed {line}, wanted {f}");
    }
    assert_eq!(lines.len(), 8);
}
