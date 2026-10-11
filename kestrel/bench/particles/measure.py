"""Runs particles or voxel viewer programs for about 11 seconds each and summarises the lines they print.

    python measure.py [--rounds 3] [--secs 11] NAME=COMMAND...

Each argument is a name, `=`, and a command line (leading `KEY=VALUE` words are environment
variables). Every round runs each command once, in the order given, so the commands share the
machine's conditions; each run is stopped after --secs seconds by killing the process this script
started. For example (from kestrel/, after building the Rust crate and the Kestrel CLI):

    python bench/particles/measure.py --rounds 3 \
      "kestrel-jit=KESTREL_HIDDEN=1 target/release/kestrel.exe run --release my_copy.kes" \
      "rust-direct=bench/particles/rust/target/release/particles-rust --count 200000 --draw direct"

Method, as in demos/particles/results.md: the first line a program prints (one a second, it holds
start-up) is dropped, and a cell is the median of the per-second averages, with the range of those
in brackets. Over several rounds the values of all of them are pooled. A line is the demo's
`particles N update X ms draw Y ms frame Z ms fps F`, optionally followed by the Rust port's extra
columns `copy`, `fill`, `upload` and `drawmesh` (ms) and `moves` (CPU changes per frame).

The voxel viewer (demos/voxels/voxels.kes) prints `voxels W meshes M faces F draw D ms frame T ms
fps P` once a second, and one line at start, `voxels world W chunks C meshes M faces F build B ms
upload U ms`. Its per-second lines are treated like the particles demo's (columns `draw`, `frame`,
`fps`; the first dropped). The start line comes once a run, so its `build` and `upload` are
summarised as a median with the range over the runs, in the same row. A run whose program prints
voxel lines gets the voxels table (and the particles table lists only the other runs). The warning
about runs held to 60 fps and `--retry-capped` work for both; for the viewer a run is flagged when its
median frame is between 16.4 and 17.0 ms (a slower frame is not the display's cap).

Every command must open its window hidden (`KESTREL_HIDDEN=1` for Kestrel programs; the Rust port
always does). This script never opens a window itself.
"""

import argparse
import os
import re
import shlex
import statistics
import subprocess
import threading

LINE = re.compile(
    r"^particles (\d+) update ([\d.]+) ms draw ([\d.]+) ms frame ([\d.]+) ms fps ([\d.]+)"
    r"(?: copy ([\d.]+) ms fill ([\d.]+) ms upload ([\d.]+) ms drawmesh ([\d.]+) ms(?: moves ([\d.]+))?)?\s*$"
)
COLUMNS = ["update", "draw", "frame", "fps", "copy", "fill", "upload", "drawmesh", "moves"]
# The voxel viewer's per-second line, and its start line (one a run).
VOXEL_LINE = re.compile(r"^voxels (\d+) meshes (\d+) faces (\d+) draw ([\d.]+) ms frame ([\d.]+) ms fps ([\d.]+)\s*$")
VOXEL_START = re.compile(r"^voxels world (\d+) chunks (\d+) meshes (\d+) faces (\d+) build ([\d.]+) ms upload ([\d.]+) ms\s*$")
VOXEL_START_COLUMNS = ["width", "chunks", "meshes", "faces", "build", "upload"]


def run_once(command, env, secs):
    """The per-second lines (a dict of columns each, first dropped) and the viewer's start line (a dict, or None)
    the command printed in `secs` seconds. Voxel lines have the columns `voxels` (their W), `draw`, `frame` and `fps`."""
    proc = subprocess.Popen(command, env=env, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True)
    timer = threading.Timer(secs, proc.kill)
    timer.start()
    rows = []
    start = None
    for line in proc.stdout:
        m = LINE.match(line)
        if m:
            values = [float(v) if v is not None else None for v in m.groups()[1:]]
            rows.append(dict(zip(COLUMNS, values)))
            continue
        m = VOXEL_LINE.match(line)
        if m:
            _, _, _, draw, frame, fps = (float(v) for v in m.groups())
            rows.append({"voxels": True, "draw": draw, "frame": frame, "fps": fps})
            continue
        m = VOXEL_START.match(line)
        if m:
            start = dict(zip(VOXEL_START_COLUMNS, (float(v) for v in m.groups())))
    proc.wait()
    timer.cancel()
    return rows[1:], start  # the first second holds start-up


def held_to_60(row):
    """Whether a per-second line looks like the display's 60 fps cap (a 16.67 ms frame)."""
    return 16.4 <= row["frame"] <= 17.0 and 58.0 <= row["fps"] <= 62.0


def parse(spec):
    name, _, line = spec.partition("=")
    words = shlex.split(line, posix=True)
    env = dict(os.environ)
    for key in [k for k in list(env) if k.startswith("KESTREL_")]:
        del env[key]  # the caller's settings must not leak into a run
    while words and re.match(r"^[A-Z_]+=", words[0]):
        key, _, value = words.pop(0).partition("=")
        env[key] = value
    if "/" in words[0] or "\\" in words[0]:
        words[0] = os.path.abspath(words[0])  # CreateProcess won't find `dir/prog.exe` relative to here
    return name, words, env


def cell(values, digits=2):
    values = [v for v in values if v is not None]
    if not values:
        return "-"
    f = f"{{:.{digits}f}}"
    return f"{f.format(statistics.median(values))} ({f.format(min(values))}-{f.format(max(values))})"


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--rounds", type=int, default=3)
    ap.add_argument("--secs", type=float, default=11.0)
    ap.add_argument("--retry-capped", type=int, default=0, metavar="N",
                    help="run again (up to N times) a run that has a line held to 60 fps; only for counts whose frame is well under 16 ms")
    ap.add_argument("--raw", action="store_true",
                    help="also print every run's per-second update/draw/frame (the viewer's: its start line's build and upload, then draw/frame)")
    ap.add_argument("configs", nargs="+")
    args = ap.parse_args()
    configs = [parse(c) for c in args.configs]
    rows = {name: [] for name, _, _ in configs}
    per_run = {name: [] for name, _, _ in configs}
    starts = {name: [] for name, _, _ in configs}  # the viewer's start line of each kept run
    discarded = {name: 0 for name, _, _ in configs}
    for round_no in range(args.rounds):
        for name, command, env in configs:
            run, start = run_once(command, env, args.secs)
            for _ in range(args.retry_capped):
                if not any(held_to_60(r) for r in run):
                    break
                discarded[name] += 1
                run, start = run_once(command, env, args.secs)
            rows[name] += run
            per_run[name].append(run)
            if start:
                starts[name].append(start)
            if args.raw:
                if any(r.get("voxels") for r in run):
                    build = f"build {start['build']} upload {start['upload']} | " if start else ""
                    print(f"[round {round_no + 1}] {name}: " + build + " | ".join(f"{r['draw']}/{r['frame']}" for r in run))
                else:
                    print(f"[round {round_no + 1}] {name}: " + " | ".join(f"{r['update']}/{r['draw']}/{r['frame']}" for r in run))
    is_voxels = {name: bool(starts[name]) or any(x.get("voxels") for x in rows[name]) for name in rows}
    if not all(is_voxels.values()):
        print("| Run | values | update ms | draw ms | frame ms | fps | copy ms | fill ms | upload ms | moves |")
        print("|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|")
        for name in rows:
            if is_voxels[name]:
                continue
            r = rows[name]
            if not r:
                print(f"| {name} | 0 | no lines |")
                continue
            digits = {"fps": 1}
            cells = [cell([x[c] for x in r], digits.get(c, 2)) for c in ["update", "draw", "frame", "fps", "copy", "fill", "upload", "moves"]]
            print(f"| {name} | {len(r)} | " + " | ".join(cells) + " |")
    if any(is_voxels.values()):
        if not all(is_voxels.values()):
            print()
        print("| Run | values | draw ms | frame ms | fps | runs | build ms | upload ms | meshes | faces |")
        print("|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|")
        for name in rows:
            if not is_voxels[name]:
                continue
            r, s = rows[name], starts[name]
            cells = [cell([x[c] for x in r], 1 if c == "fps" else 2) for c in ["draw", "frame", "fps"]]
            start_cells = [cell([x[c] for x in s], 2 if c in ("build", "upload") else 0) for c in ["build", "upload", "meshes", "faces"]]
            print(f"| {name} | {len(r)} | " + " | ".join(cells) + f" | {len(s)} | " + " | ".join(start_cells) + " |")
    for name in rows:
        if is_voxels[name]:
            capped = [i + 1 for i, run in enumerate(per_run[name]) if run and 16.4 <= statistics.median(x["frame"] for x in run) <= 17.0]
        else:
            capped = [i + 1 for i, run in enumerate(per_run[name]) if run and statistics.median(x["frame"] for x in run) > 16.4]
        if capped:
            print(f"WARNING: {name} run(s) {capped} were held to 60 fps (a 16.67 ms frame): discard them (see results.md)")
    for name, count in discarded.items():
        if count:
            print(f"{name}: {count} run(s) held to 60 fps were thrown away and repeated")
    print()
    print("Median draw / frame ms of each run:")
    for name in rows:
        medians = [
            f"{statistics.median(x['draw'] for x in run):.2f}/{statistics.median(x['frame'] for x in run):.2f}"
            for run in per_run[name]
            if run
        ]
        print(f"  {name}: " + "  ".join(medians))


if __name__ == "__main__":
    main()
