"""Runs every kernel, Kestrel and Rust, for a few interleaved rounds and prints the median of the
rounds' medians, with their range, in ns per item.

    py measure.py [--rounds 3] [--threads 1] [--only TEXT] [--keep-checks] [--old DIR]

Run from kestrel/bench/kernels after `cargo build --release` there. `--threads` sets
KESTREL_THREADS for the Kestrel kernels (their `par for`); the Rust kernels are one thread.
`--only TEXT` runs just the kernels whose label contains TEXT (case-insensitive), e.g. `--only iface`
for the interface kernels, Kestrel and Rust.

`--rust-checks DIR` adds a column for the Rust kernels, run from DIR: a build with overflow checks
(`RUSTFLAGS="-C overflow-checks=on" cargo build --release --target-dir target/ovf`, then
`--rust-checks target/ovf/release`). The race kernels (labels with "race") must print the face
total 463,749 as their checksum, or the run stops.

`--keep-checks` and `--old` add columns, run in turns with the first within each round:
`--keep-checks` is the same compiler with `remove_checks: false` (every check the range analysis
removes stays); `--old DIR` is a `bench/kernels` build made with an older compiler (its
`target/release`, e.g. one built in a `git worktree` of the commit before the analysis). The Rust
kernels are run once per round, whatever the columns.
"""
import argparse, os, re, statistics, subprocess

HERE = os.path.dirname(os.path.abspath(__file__))
BIN = os.path.join(HERE, "target", "release")

# (label, command, iterations): more iterations for the cheap kernels.
KERNELS = [
    ("step.kes (the demo's step, pow hoisted)", ["kernels", "step.kes", "--release"], 30),
    ("step_index.kes (step in a for i loop over parts[i])", ["kernels", "step_index.kes", "--release"], 30),
    ("step_scalar.kes (written with float locals)", ["kernels", "step_scalar.kes", "--release"], 30),
    ("step_pow.kes (pow per particle, the demo)", ["kernels", "step_pow.kes", "--release"], 15),
    ("copy.kes (the draw copy)", ["kernels", "copy.kes", "--release"], 100),
    ("filter.kes (xs.filter, 1M ints)", ["kernels", "filter.kes", "--release", "--n", "1000000"], 10),
    ("filter_loop.kes (hand loop)", ["kernels", "filter_loop.kes", "--release", "--n", "1000000"], 10),
    ("map.kes (xs.map, 1M vec2 lengths)", ["kernels", "map.kes", "--release", "--n", "1000000"], 10),
    ("map_loop.kes (hand loop)", ["kernels", "map_loop.kes", "--release", "--n", "1000000"], 10),
    ("sort_by.kes (xs.sort_by, 100k structs)", ["kernels", "sort_by.kes", "--release", "--n", "100000"], 10),
    ("sort_by_loop.kes (hand merge sort)", ["kernels", "sort_by_loop.kes", "--release", "--n", "100000"], 10),
    ("iface_hp.kes (hp() through list[Damageable], 1M)", ["kernels", "iface_hp.kes", "--release", "--n", "1000000"], 20),
    ("iface_hp_direct.kes (list[Goblin])", ["kernels", "iface_hp_direct.kes", "--release", "--n", "1000000"], 20),
    ("iface_step.kes (step(var self) through list[Damageable])", ["kernels", "iface_step.kes", "--release", "--n", "1000000"], 20),
    ("iface_step_direct.kes (list[Goblin])", ["kernels", "iface_step_direct.kes", "--release", "--n", "1000000"], 20),
    ("iface_step_generic.kes (step_all[T], list[Goblin])", ["kernels", "iface_step_generic.kes", "--release", "--n", "1000000"], 20),
    # The value noise, 262,144 samples (256 rows of 1,024) per pass; checked and `--release`.
    ("noise_scalar.kes checked (four hash calls)", ["kernels", "noise_scalar.kes", "--n", "262144"], 10),
    ("noise_scalar.kes --release", ["kernels", "noise_scalar.kes", "--release", "--n", "262144"], 10),
    ("noise_corners.kes checked (one hash4 for the corners)", ["kernels", "noise_corners.kes", "--n", "262144"], 10),
    ("noise_corners.kes --release", ["kernels", "noise_corners.kes", "--release", "--n", "262144"], 10),
    ("noise_wide.kes checked (four samples at once)", ["kernels", "noise_wide.kes", "--n", "262144"], 10),
    ("noise_wide.kes --release", ["kernels", "noise_wide.kes", "--release", "--n", "262144"], 10),
    ("Rust step", ["rust", "step"], 30),
    ("Rust pow", ["rust", "pow"], 15),
    ("Rust copy", ["rust", "copy"], 100),
    ("Rust filter", ["rust", "filter", "--n", "1000000"], 10),
    ("Rust map", ["rust", "map", "--n", "1000000"], 10),
    ("Rust sort_by", ["rust", "sort_by", "--n", "100000"], 10),
    ("Rust iface dyn hp (Vec<Box<dyn Trait>>)", ["rust", "dyn_hp", "--n", "1000000"], 20),
    ("Rust iface direct hp (Vec<Goblin>)", ["rust", "direct_hp", "--n", "1000000"], 20),
    ("Rust iface dyn step (Vec<Box<dyn Trait>>)", ["rust", "dyn_step", "--n", "1000000"], 20),
    ("Rust iface direct step (Vec<Goblin>)", ["rust", "direct_step", "--n", "1000000"], 20),
    ("Rust noise_scalar", ["rust", "noise_scalar", "--n", "262144"], 10),
    ("Rust noise_wide (SSE4.1)", ["rust", "noise_wide", "--n", "262144"], 10),
    # The race against block-mesh (E26), ns per chunk of the frozen world's 256 chunks: Kestrel
    # builds the chunks' solid flags in each call (and the timer takes that out), Rust once. Each
    # prints the face total of a pass as its checksum, and it must be 463,749 (checked below).
    ("race_faces.kes checked (six lists of packed faces)", ["kernels", "../../voxel/race_faces.kes", "--n", "256", "--reps", "5"], 60),
    ("race_faces.kes --release", ["kernels", "../../voxel/race_faces.kes", "--release", "--n", "256", "--reps", "5"], 60),
    ("race_arrays.kes checked (faces, then positions, normals, indices)", ["kernels", "../../voxel/race_arrays.kes", "--n", "256", "--reps", "5"], 40),
    ("race_arrays.kes --release", ["kernels", "../../voxel/race_arrays.kes", "--release", "--n", "256", "--reps", "5"], 40),
    ("Rust race_faces (block-mesh visible_block_faces)", ["rust", "race_faces", "--n", "256", "--reps", "5"], 60),
    ("Rust race_arrays (visible_block_faces, quad_mesh_*)", ["rust", "race_arrays", "--n", "256", "--reps", "5"], 40),
    # Not part of the race: race.kes's kernels written out in Rust (checked indexing, six Vec<u32>s).
    ("Rust race_faces_plain (race.kes's kernel in Rust)", ["rust", "race_faces_plain", "--n", "256", "--reps", "5"], 60),
    ("Rust race_arrays_plain (race.kes's kernels in Rust)", ["rust", "race_arrays_plain", "--n", "256", "--reps", "5"], 40),
]
RACE_FACES = 463749


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--rounds", type=int, default=3)
    ap.add_argument("--threads", type=int, default=1)
    ap.add_argument("--only", default="")
    ap.add_argument("--keep-checks", action="store_true", help="also run with every proven check kept")
    ap.add_argument("--old", default="", help="also run the kernels built in DIR (an older compiler's target/release)")
    ap.add_argument("--rust-checks", default="", help="also run the Rust kernels built in DIR (with overflow checks on)")
    a = ap.parse_args()
    env = dict(os.environ, KESTREL_THREADS=str(a.threads))
    # (column, directory of the binaries, extra flags for the Kestrel kernels)
    columns = [("new", BIN, [])]
    if a.keep_checks:
        columns.append(("new --keep-checks", BIN, ["--keep-checks"]))
    if a.old:
        columns.append(("old", os.path.abspath(a.old), []))
    if a.rust_checks:
        columns.append(("overflow checks", os.path.abspath(a.rust_checks), []))
    kernels = [k for k in KERNELS if a.only.lower() in k[0].lower()]
    results = {(label, c[0]): [] for label, _, _ in kernels for c in columns}
    for _ in range(a.rounds):
        for label, cmd, iters in kernels:
            for name, directory, extra in columns:
                is_rust = cmd[0] == "rust"
                if is_rust and name not in ("new", "overflow checks"):
                    continue
                if not is_rust and name == "overflow checks":
                    continue
                exe = os.path.join(directory, cmd[0] + (".exe" if os.name == "nt" else ""))
                args = [exe, *cmd[1:], *([] if is_rust else extra), "--iters", str(iters)]
                out = subprocess.run(args, cwd=HERE, env=env, capture_output=True, text=True)
                m = re.search(r"([\d.]+) ns per item", out.stdout)
                if not m:
                    raise SystemExit(f"{label} ({name}) failed:\n{out.stdout}{out.stderr}")
                if "race" in label.lower():
                    total = re.search(r"checksum ([\d.]+)", out.stdout)
                    if not total or float(total.group(1)) != RACE_FACES:
                        raise SystemExit(f"{label} ({name}) found {total and total.group(1)} faces, not {RACE_FACES}:\n{out.stdout}")
                results[(label, name)].append(float(m.group(1)))
    print(f"{a.threads} thread(s) for Kestrel, {a.rounds} rounds")
    for label, _, _ in kernels:
        cells = []
        for name, _, _ in columns:
            vals = results[(label, name)]
            if vals:
                cells.append(f"{name} {statistics.median(vals):6.2f} ns ({min(vals):.2f}-{max(vals):.2f})")
        print(f"{label:56s} " + "   ".join(cells))


if __name__ == "__main__":
    main()
