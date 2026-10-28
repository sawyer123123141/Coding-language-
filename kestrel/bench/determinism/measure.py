"""The determinism round's measurements (`results.md`, "Determinism round 1"): two or more Kestrel
compilers, run in turn round by round, with the machine's load sampled before, during and after
every block.

    py measure.py --kestrel before=DIR after=DIR [host=DIR] --kernels before=DIR ... [--out DIR] CMD [N]

`--kestrel name=DIR` names a compiler by the folder holding its `kestrel.exe` (a `target/release`);
`--kernels name=DIR` the folder holding the `bench/kernels` harness built with it (`kernels.exe`,
`rust.exe`). The first name is the reference. Commands (N rounds, default 9):

  build       builds every exe below with every compiler into `--out` (default `out/`)
  world N     the frozen world (`bench/voxels3d/main.kes`), checked and `--release`
  gen N       generation: 64 columns through `world.load_now`, one thread, `--release`
  jit N       `kestrel run --release` of the frozen world and of generation (compile included)
  snap N      `snapshot.kes`: save and restore of Pong's `Game` and of a voxel-sized `Pool`, and
              `checksum` of both (only for compilers that have it; the first one must not)
  kernels N --only TEXT ...   `bench/kernels` rows (through the JIT), every compiler in turn
  engine N    `bench/engine` (`voxel/bench.kes`): N runs per compiler in turn, p99 per phase
  math N --test-exe EXE   the runtime's speed tests: `libm` against the platform, and `fpu::pin`

Exes run at normal priority, with the matching empty program's median (process start-up, and for
`snap` the state's set-up) subtracted. "During" is the whole machine's CPU use over the block,
the timed processes included (one busy core of 16 is 6.25%). Every exe's output must be the same
for every compiler, or the block stops.
"""
import argparse, ctypes, os, re, shutil, statistics, subprocess, sys, time

HERE = os.path.dirname(os.path.abspath(__file__))
KESTREL_DIR = os.path.normpath(os.path.join(HERE, "..", ".."))
VOXEL = os.path.join(KESTREL_DIR, "voxel")
WORLD = os.path.join(KESTREL_DIR, "bench", "voxels3d")
KERNELS = os.path.join(KESTREL_DIR, "bench", "kernels")

GEN64 = """use { new_world, load_now from world }

main() {
    var w = new_world(1337, 64)
    for cz from 0 to 8 {
        for cx from 0 to 8 {
            load_now(var w, cx, cz)
        }
    }
    var sum = 0
    for x from 0 to 256 {
        for y from 0 to 128 {
            sum += int(w.block(x, y, x / 2))
        }
    }
    print(sum)
}
"""
EMPTY = "main() {\n    print(0)\n}\n"
# Snapshot programs: (name, call, needs checksum). The `n` 0 ones are the set-up to take out.
SNAP = [
    ("pong_1m", "pong_pairs(1000000)", False), ("pongc_1m", "pong_control(1000000)", False),
    ("pong_100m", "pong_pairs(100000000)", False), ("pongc_100m", "pong_control(100000000)", False),
    ("pool_0", "pool_pairs(0)", False), ("pool_10", "pool_pairs(10)", False), ("pool_100", "pool_pairs(100)", False),
]
CHECKSUM = {
    "chk_game_1m": "var g = new_game()\n    var h: i64 = 0\n    for i from 0 to 1000000 {\n        nudge(var g, i)\n"
                   "        h = h ^ checksum(g)\n    }\n    print(h)",
    "chk_gamec_1m": "var g = new_game()\n    var h: i64 = 0\n    for i from 0 to 1000000 {\n        nudge(var g, i)\n"
                    "        h = h ^ g.rng.seed\n    }\n    print(h)",
    "chk_pool_20": "var p = new_pool()\n    var h: i64 = 0\n    for i from 0 to 20 {\n        p.items[i].blocks.items[i] = 1\n"
                   "        h = h ^ checksum(p)\n    }\n    print(h)",
    "chk_pool_0": "var p = new_pool()\n    var h: i64 = 0\n    for i from 0 to 0 {\n        p.items[i].blocks.items[i] = 1\n"
                  "        h = h ^ checksum(p)\n    }\n    print(h)",
}
# (timed program, its set-up program) per block.
PAIRS = {
    "world": [("world", "world0"), ("world_r", "world0")],
    "gen": [("gen64", "gen0")],
    "snap": [("pong_1m", "pongc_1m"), ("pong_100m", "pongc_100m"), ("pool_10", "pool_0"), ("pool_100", "pool_0"),
             ("chk_game_1m", "chk_gamec_1m"), ("chk_pool_20", "chk_pool_0")],
}
# Programs the first compiler (from before the round) can't run: too much memory without reuse
# (pool_100 would take 100 x 2 x 58 MB), or no `checksum`.
NOT_FIRST = {"pool_100", "chk_game_1m", "chk_gamec_1m", "chk_pool_20", "chk_pool_0"}


class FT(ctypes.Structure):
    _fields_ = [("lo", ctypes.c_uint32), ("hi", ctypes.c_uint32)]


def _times():
    i, k, u = FT(), FT(), FT()
    ctypes.windll.kernel32.GetSystemTimes(ctypes.byref(i), ctypes.byref(k), ctypes.byref(u))
    f = lambda t: (t.hi << 32) | t.lo
    return f(i), f(k), f(u)


def load(sec=2.0):
    """The whole machine's CPU use over `sec` seconds, in percent."""
    a = _times()
    time.sleep(sec)
    return During.pct_between(a, _times())


class During:
    """The whole machine's CPU use between enter and exit (the timed processes included)."""

    @staticmethod
    def pct_between(a, b):
        idle, kern, user = (b[j] - a[j] for j in range(3))
        return 100.0 * (kern + user - idle) / (kern + user) if kern + user else 0.0

    def __enter__(self):
        self.a = _times()
        return self

    def __exit__(self, *e):
        self.pct = During.pct_between(self.a, _times())


def env(threads=None):
    e = {k: v for k, v in os.environ.items() if not k.startswith("KESTREL_")}
    if threads:
        e["KESTREL_THREADS"] = str(threads)
    return e


def run(cmd, cwd=None, threads=None):
    t = time.perf_counter()
    r = subprocess.run(cmd, cwd=cwd, env=env(threads), capture_output=True, text=True)
    dt = (time.perf_counter() - t) * 1000
    if r.returncode != 0:
        sys.exit(f"{' '.join(cmd)} exited {r.returncode}:\n{r.stdout[-2000:]}{r.stderr[-2000:]}")
    return dt, r.stdout.strip()


def spread(xs):
    d = 2 if statistics.median(xs) < 100 else 1
    return f"{statistics.median(xs):.{d}f} ({min(xs):.{d}f}-{max(xs):.{d}f})"


def exe(a, name, v):
    return os.path.join(a.out, f"{name}-{v}.exe")


def cmd_build(a):
    gen, snap = os.path.join(a.out, "gen"), os.path.join(a.out, "snap")
    for d in (gen, snap):
        os.makedirs(d, exist_ok=True)
    for f in os.listdir(VOXEL):
        if f.endswith(".kes"):
            shutil.copy(os.path.join(VOXEL, f), gen)
    open(os.path.join(gen, "gen64.kes"), "w", newline="\n").write(GEN64)
    open(os.path.join(gen, "gen0.kes"), "w", newline="\n").write(EMPTY)
    shutil.copy(os.path.join(HERE, "snapshot.kes"), snap)
    names = []
    for name, call, _ in SNAP:
        open(os.path.join(snap, f"{name}.kes"), "w", newline="\n").write(
            f"use {{ pong_pairs, pong_control, pool_pairs from snapshot }}\n\nmain() {{\n    print({call})\n}}\n")
        names.append(name)
    for name, body in CHECKSUM.items():
        open(os.path.join(snap, f"{name}.kes"), "w", newline="\n").write(
            f"use {{ new_game, nudge, new_pool from snapshot }}\n\nmain() {{\n    {body}\n}}\n")
    progs = [("world", os.path.join(WORLD, "main.kes"), []), ("world_r", os.path.join(WORLD, "main.kes"), ["--release"]),
             ("world0", os.path.join(WORLD, "main0.kes"), ["--release"]),
             ("gen64", os.path.join(gen, "gen64.kes"), ["--release"]), ("gen0", os.path.join(gen, "gen0.kes"), ["--release"])]
    progs += [(n, os.path.join(snap, f"{n}.kes"), ["--release"]) for n in names + list(CHECKSUM)]
    for i, v in enumerate(a.kestrel):
        for name, src, flags in progs:
            if i == 0 and name in NOT_FIRST:
                continue
            t, _ = run([a.kestrel[v], "build", src, "-o", exe(a, name, v), *flags], cwd=os.path.dirname(src))
            print(f"built {name}-{v} in {t:.0f} ms", flush=True)


def timed_block(a, title, jobs, rounds):
    """`jobs`: {(variant, name): command}. Runs every command once per round, in order; prints the
    load and returns {(variant, name): [ms]} and the outputs."""
    times, outs = {k: [] for k in jobs}, {k: set() for k in jobs}
    before = load()
    with During() as d:
        for _ in range(rounds):
            for k, cmd in jobs.items():
                dt, out = run(cmd, cwd=a.out)
                times[k].append(dt)
                outs[k].add(out)
    print(f"{title}: {rounds} rounds; load before {before:.1f}%, during {d.pct:.1f}%, after {load():.1f}%", flush=True)
    return times, outs


def report(a, times, outs, pairs, per=None):
    variants = list(a.kestrel)
    for prog, setup in pairs:
        printed = {o for (v, n), s in outs.items() if n == prog for o in s}
        if len(printed) != 1:
            sys.exit(f"{prog} printed different things: {outs}")
        printed, ref = printed.pop(), None
        for v in variants:
            if (v, prog) not in times:
                continue
            t, s = times[(v, prog)], times[(v, setup)]
            net = statistics.median(t) - statistics.median(s)
            ref = ref or net
            unit = f", {net / per[prog] * 1e6:.2f} ns each" if per and prog in per else ""
            print(f"  {prog:12} {v:7} whole {spread(t)} ms - set-up {setup} {spread(s)} ms = {net:.1f} ms"
                  f" ({net / ref:.3f} of {next(x for x in variants if (x, prog) in times)}){unit}; prints {printed}")
        print("  raw", {f"{v}:{n}": [round(x, 1) for x in times[(v, n)]] for v in variants for n in (prog, setup)
                        if (v, n) in times}, flush=True)


def cmd_exes(a, block, rounds):
    names = sorted({n for p in PAIRS[block] for n in p})
    jobs = {(v, n): [exe(a, n, v)] for i, v in enumerate(a.kestrel) for n in names if not (i == 0 and n in NOT_FIRST)}
    times, outs = timed_block(a, block, jobs, rounds)
    per = {"pong_1m": 1e6, "pong_100m": 1e8, "pool_10": 10, "pool_100": 100, "chk_game_1m": 1e6, "chk_pool_20": 20}
    report(a, times, outs, PAIRS[block], per if block == "snap" else None)
    if block == "snap":
        peak(a)


def peak(a):
    """Peak private bytes of the pool programs (one more run each), to show the copies' memory."""
    sys.path.insert(0, os.path.join(KESTREL_DIR, "bench", "engine"))
    import measure as eng
    for i, v in enumerate(a.kestrel):
        for n in ("pool_0", "pool_10", "pool_100"):
            if i == 0 and n in NOT_FIRST:
                continue
            p = subprocess.Popen([exe(a, n, v)], stdout=subprocess.DEVNULL, env=env())
            h = eng.kernel32.OpenProcess(eng.PROCESS_QUERY_INFORMATION | eng.PROCESS_VM_READ, False, p.pid)
            p.wait()
            pmc = eng.PROCESS_MEMORY_COUNTERS_EX()
            pmc.cb = ctypes.sizeof(pmc)
            eng.psapi.GetProcessMemoryInfo(h, ctypes.byref(pmc), pmc.cb)
            print(f"  peak {n:9} {v:7} private {pmc.PeakPagefileUsage / 2**20:8.1f} MB, working set "
                  f"{pmc.PeakWorkingSetSize / 2**20:8.1f} MB", flush=True)


def cmd_jit(a, rounds):
    gen = os.path.join(a.out, "gen")
    progs = {"world": (os.path.join(WORLD, "main.kes"), WORLD), "world0": (os.path.join(WORLD, "main0.kes"), WORLD),
             "gen64": (os.path.join(gen, "gen64.kes"), gen), "gen0": (os.path.join(gen, "gen0.kes"), gen)}
    jobs = {(v, n): [a.kestrel[v], "run", "--release", src] for v in a.kestrel for n, (src, _) in progs.items()}
    times, outs = timed_block(a, "jit (kestrel run --release, compile included)", jobs, rounds)
    report(a, times, outs, [("world", "world0"), ("gen64", "gen0")])


def cmd_kernels(a, rounds, onlys):
    sys.path.insert(0, KERNELS)
    import measure as km
    for only in onlys:
        rows = [k for k in km.KERNELS if only.lower() in k[0].lower()]
        res = {}
        before = load()
        with During() as d:
            for _ in range(rounds):
                for label, cmd, iters in rows:
                    cols = list(a.kernels)[:1] if cmd[0] == "rust" else list(a.kernels)
                    for v in cols:
                        args = [os.path.join(a.kernels[v], cmd[0] + ".exe"), *cmd[1:], "--iters", str(iters)]
                        _, out = run(args, cwd=KERNELS, threads=1)
                        m = re.search(r"([\d.]+) ns per item", out)
                        if "race" in label.lower():
                            total = re.search(r"checksum ([\d.]+)", out)
                            assert total and float(total.group(1)) == km.RACE_FACES, out
                        res.setdefault((label, v), []).append(float(m.group(1)))
        print(f"kernels --only {only!r}: {rounds} rounds, 1 thread; load before {before:.1f}%, during {d.pct:.1f}%, "
              f"after {load():.1f}%", flush=True)
        for label, cmd, _ in rows:
            cells, ref = [], None
            for v in a.kernels:
                if (label, v) in res:
                    m = statistics.median(res[(label, v)])
                    ref = ref or m
                    cells.append(f"{v} {spread(res[(label, v)])} ns ({m / ref:.3f})")
            print(f"  {label:56s} " + "   ".join(cells))
        print("  raw", {f"{v}:{l[:20]}": res[(l, v)] for (l, v) in res}, flush=True)


def cmd_engine(a, runs):
    sys.path.insert(0, os.path.join(KESTREL_DIR, "bench", "engine"))
    import measure as eng
    eng.IDLE_PRIORITY_CLASS = 0x00000020  # normal priority, like every other block here
    for v, d in a.kestrel.items():
        eng.KESTREL, eng.OUT = os.path.join(d, "kestrel.exe"), os.path.join(a.out, f"engine-{v}")
        eng.build()
    got = {v: [] for v in a.kestrel}
    for n in range(runs):
        for v in a.kestrel:
            eng.OUT = os.path.join(a.out, f"engine-{v}")
            r = eng.run(os.path.join(eng.OUT, "bench.exe"), os.path.join(eng.OUT, f"run_{n + 1}.txt"))
            got[v].append(r)
            print(f"engine {v} run {n + 1}: {r['seconds']:.1f} s, load before {r['load_before']:.1f}%, others during "
                  f"{r['load_others_during']:.1f}%", flush=True)
    for ph in ("fill", "fly20", "fly100", "edits", "circle"):
        for k in ("work_med", "work_p99", "work_max", "gen_ms", "mesh_ms"):
            print(f"  {ph:6} {k:9} " + "   ".join(f"{v} {spread([r['phases'][ph][k] for r in rs])}" for v, rs in got.items()))
    for k in ("fill_ms", "mid_ms", "corner_ms"):
        print(f"  {k:16} " + "   ".join(f"{v} {spread([r[k] for r in rs])}" for v, rs in got.items()))


def cmd_math(a, rounds):
    """The runtime's two ignored speed tests (`runtime/src/math/speed.rs`), `rounds` times each in
    turn, from the test exe `--test-exe` (`cargo test -p kestrel-runtime --release --no-run`)."""
    vals = {}
    before = load()
    with During() as d:
        for _ in range(rounds):
            for test in ("own_math_against_the_platform", "pinning_the_float_mode"):
                _, out = run([a.test_exe, test, "--ignored", "--nocapture", "--test-threads=1"])
                for name, own, plat in re.findall(r"(\w+): libm ([\d.]+) ns, platform ([\d.]+) ns", out):
                    vals.setdefault((name, "libm"), []).append(float(own))
                    vals.setdefault((name, "platform"), []).append(float(plat))
                for k, v in re.findall(r"(empty loop|pin inlined|kestrel_rt_fpu_pin called) ([\d.]+) ns", out):
                    vals.setdefault((k, ""), []).append(float(v))
    print(f"math: {rounds} rounds; load before {before:.1f}%, during {d.pct:.1f}%, after {load():.1f}%")
    for name in dict.fromkeys(n for n, _ in vals):
        if (name, "libm") in vals:
            own, plat = vals[(name, "libm")], vals[(name, "platform")]
            print(f"  {name:6} libm {spread(own)} ns, platform {spread(plat)} ns, ratio "
                  f"{statistics.median(own) / statistics.median(plat):.2f}")
        else:
            print(f"  {name:26} {spread(vals[(name, '')])} ns per iteration")
    print("  raw", {f"{n}:{k}": v for (n, k), v in vals.items()}, flush=True)


def named(pairs):
    return dict(p.split("=", 1) for p in pairs)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--kestrel", nargs="+", required=True, metavar="NAME=DIR")
    ap.add_argument("--kernels", nargs="+", default=[], metavar="NAME=DIR")
    ap.add_argument("--out", default=os.path.join(HERE, "out"))
    ap.add_argument("--only", nargs="+", default=["race", "noise", "step"])
    ap.add_argument("--test-exe", help="for `math`: the kestrel-runtime test exe, built --release")
    ap.add_argument("cmd")
    ap.add_argument("n", type=int, nargs="?", default=9)
    a = ap.parse_args()
    a.kestrel = {k: os.path.join(d, "kestrel.exe") for k, d in named(a.kestrel).items()}
    a.kernels = named(a.kernels)
    os.makedirs(a.out, exist_ok=True)
    if a.cmd == "build":
        cmd_build(a)
    elif a.cmd in ("world", "gen", "snap"):
        cmd_exes(a, a.cmd, a.n)
    elif a.cmd == "math":
        cmd_math(a, a.n)
    elif a.cmd == "jit":
        cmd_jit(a, a.n)
    elif a.cmd == "kernels":
        cmd_kernels(a, a.n, a.only)
    elif a.cmd == "engine":
        a.kestrel = {k: os.path.dirname(p) for k, p in a.kestrel.items()}
        cmd_engine(a, a.n)
    else:
        sys.exit(f"unknown command {a.cmd}")


if __name__ == "__main__":
    main()
