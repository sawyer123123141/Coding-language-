//! The demos still compile and run. Each runs in a headless window for a short while, so no
//! window opens and no graphics are needed.

use std::path::Path;
use std::process::Command;

mod common;

#[test]
fn the_particles_demo_runs_headless_and_reports_its_times() {
    let demo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../demos/particles/particles.kes");
    // The headless clock runs at 60 frames a second, and the demo prints a line once a second,
    // so 70 frames give exactly one line.
    let out = common::hermetic(Command::new(env!("CARGO_BIN_EXE_kestrel")).arg("run").arg("--release").arg(&demo))
        .env("KESTREL_HEADLESS", "70")
        .env("KESTREL_HEADLESS_QUIET", "1")
        .output()
        .unwrap();
    let (stdout, stderr) = (String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    assert!(out.status.success(), "exit {:?}\n{stderr}", out.status.code());
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines.len(), 1, "expected one report line, got:\n{stdout}");
    assert!(lines[0].starts_with("particles 50000 update "), "unexpected report line: {}", lines[0]);
    assert!(lines[0].ends_with(" fps 60"), "the headless clock is 60 fps: {}", lines[0]);
}
