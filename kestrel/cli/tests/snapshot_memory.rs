//! A ring of snapshots stops allocating once each slot has held the longest state: assigning a
//! snapshot over an older one reuses the older one's buffers when they have room.
//!
//! Memory is measured with the runtime's own report (`KESTREL_ARENA_REPORT=1`: the size of the
//! program arena's blocks when `main` returns), which is exact. Each program runs for 2 periods
//! and for 200; a period (816 frames: 16 slots times 51 lengths) is enough for every slot to have
//! held the longest list. The control leaves out the save and restore lines, to show the program's
//! own work allocates nothing after its first period.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

mod common;

use common::hermetic;

/// The program, with `PERIODS` periods, saving and restoring snapshots when `ring` is set.
fn program(periods: u32, ring: bool) -> String {
    let (save, restore) = if ring {
        ("        ring[f % 16] = g\n", "        if f % 7 == 0 {\n            g = ring[(f + 11) % 16]\n        }\n")
    } else {
        ("", "")
    };
    format!(
        "let PERIODS = {periods}

struct State {{
    xs: list[int]
    name: string
    hp: int
}}

main() {{
    var g = State(xs: [], name: \"snapshot\", hp: 0)
    var ring: list[State] = []
    for i from 0 to 16 {{
        ring.add(State(xs: [], name: \"snapshot\", hp: 0))
    }}
    for f from 0 to PERIODS * 816 {{
        g.xs.clear()
        for k from 0 to f % 51 {{
            g.xs.add(k)
        }}
        g.hp = f
{save}{restore}    }}
    print(g.hp, g.xs.len, ring.len)
}}
"
    )
}

/// Writes `src` as `name.kes` and gives its path.
fn write(name: &str, src: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("snapshot_memory");
    fs::create_dir_all(&dir).unwrap();
    let path = dir.join(format!("{name}.kes"));
    fs::write(&path, src).unwrap();
    path
}

/// Runs the program at `path` and gives the program arena's size it reported.
fn arena_bytes(path: &Path) -> u64 {
    let out = hermetic(Command::new(env!("CARGO_BIN_EXE_kestrel")).arg("run").arg(path)).env("KESTREL_ARENA_REPORT", "1").output().unwrap();
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "{}: exit {:?}\n{stderr}", path.display(), out.status.code());
    let line = stderr
        .lines()
        .find_map(|l| l.strip_prefix("kestrel: program arena "))
        .unwrap_or_else(|| panic!("no arena report in stderr:\n{stderr}"));
    line.strip_suffix(" bytes").and_then(|n| n.parse().ok()).unwrap_or_else(|| panic!("can't read the arena report {line:?}"))
}

#[test]
fn a_snapshot_ring_stops_allocating_after_its_first_lap() {
    let bytes = |name: &str, periods: u32, ring: bool| arena_bytes(&write(name, &program(periods, ring)));
    let (control_few, control_many) = (bytes("control_2", 2, false), bytes("control_200", 200, false));
    assert_eq!(control_few, control_many, "the control's own work allocates after its first period");
    let (ring_few, ring_many) = (bytes("ring_2", 2, true), bytes("ring_200", 200, true));
    assert_eq!(ring_few, ring_many, "the snapshot ring grew from {ring_few} bytes (2 periods) to {ring_many} bytes (200 periods)");
}
