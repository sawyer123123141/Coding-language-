"""Runs the voxel engine's benchmark (`kestrel/voxel/bench.kes`) and prints its numbers: each
phase's frame times, generate, mesh and upload times, the edits' latency, the fill time, and the
process's working set over the run (E18b, E31, E32).

    py measure.py [--runs 3] [--skip-build] [--no-arena]
    py measure.py --compile 9      (compile times of main.kes instead; see `compile_times`)

Run from kestrel/bench/engine after `cargo build --release -p kestrel-cli` in kestrel/. It:

1. builds `bench.kes` with `--release` into `out/bench.exe`, and a second copy with the circle
   cut to 100 simulated seconds (`out/bench_c100.exe`, from a scratch copy of the engine in
   `out/c100/`; `kestrel/voxel` is not touched);
2. runs `out/bench.exe` `--runs` times in a hidden window (`KESTREL_HIDDEN=1`: nothing appears
   on screen), sampling the process's working set and private bytes every 250 ms
   (`GetProcessMemoryInfo`) and its dedicated GPU memory (Windows' "GPU Process Memory"
   counters, through `pdh.dll`), and timestamping every line it prints, so the `mark` lines
   place the samples in the circle (the hidden window doesn't wait for the display, so 300
   simulated seconds take about 45 real ones);
3. runs the 100 s and the 300 s copy once more each with `KESTREL_ARENA_REPORT=1`: the program
   arena's size at exit (the runtime reports it only then) must be the same for both (E31: no
   growth over the flight). The timed runs report it too.

The machine's load is sampled for 2 s before each run, and during each run as the whole
machine's CPU time minus the benchmark's own. Every run is at idle priority, so other work wins.
Raw output goes to `out/measure_<n>.txt` and everything parsed to `out/measure.json`.
"""
import argparse, ctypes, json, os, re, shutil, statistics, subprocess, sys, threading, time
from ctypes import wintypes

HERE = os.path.dirname(os.path.abspath(__file__))
KESTREL_DIR = os.path.normpath(os.path.join(HERE, "..", ".."))
KESTREL = os.path.join(KESTREL_DIR, "target", "release", "kestrel.exe")
VOXEL = os.path.join(KESTREL_DIR, "voxel")
OUT = os.path.join(HERE, "out")

IDLE_PRIORITY_CLASS = 0x00000040
CREATE_NO_WINDOW = 0x08000000
PROCESS_QUERY_INFORMATION = 0x0400
PROCESS_VM_READ = 0x0010
SAMPLE_SECONDS = 0.25

PHASE_FIELDS = ["frames", "work_med", "work_p99", "work_max", "wall_med", "generated", "meshed", "gen_ms", "mesh_ms",
                "upload_ms"]


class PROCESS_MEMORY_COUNTERS_EX(ctypes.Structure):
    _fields_ = [("cb", wintypes.DWORD), ("PageFaultCount", wintypes.DWORD), ("PeakWorkingSetSize", ctypes.c_size_t),
                ("WorkingSetSize", ctypes.c_size_t), ("QuotaPeakPagedPoolUsage", ctypes.c_size_t),
                ("QuotaPagedPoolUsage", ctypes.c_size_t), ("QuotaPeakNonPagedPoolUsage", ctypes.c_size_t),
                ("QuotaNonPagedPoolUsage", ctypes.c_size_t), ("PagefileUsage", ctypes.c_size_t),
                ("PeakPagefileUsage", ctypes.c_size_t), ("PrivateUsage", ctypes.c_size_t)]


kernel32 = ctypes.WinDLL("kernel32", use_last_error=True)
psapi = ctypes.WinDLL("psapi", use_last_error=True)
kernel32.OpenProcess.restype = wintypes.HANDLE
kernel32.OpenProcess.argtypes = [wintypes.DWORD, wintypes.BOOL, wintypes.DWORD]
psapi.GetProcessMemoryInfo.argtypes = [wintypes.HANDLE, ctypes.POINTER(PROCESS_MEMORY_COUNTERS_EX), wintypes.DWORD]
kernel32.GetSystemTimes.argtypes = [ctypes.POINTER(wintypes.FILETIME)] * 3
kernel32.GetProcessTimes.argtypes = [wintypes.HANDLE] + [ctypes.POINTER(wintypes.FILETIME)] * 4


pdh = ctypes.WinDLL("pdh")
for f in ("PdhOpenQueryW", "PdhAddEnglishCounterW", "PdhCollectQueryData", "PdhGetFormattedCounterArrayW", "PdhCloseQuery"):
    getattr(pdh, f).restype = wintypes.DWORD
PDH_FMT_LARGE = 0x00000400
PDH_MORE_DATA = 0x800007D2


class PDH_VALUE(ctypes.Structure):
    _fields_ = [("CStatus", wintypes.DWORD), ("largeValue", ctypes.c_longlong)]


class PDH_ITEM(ctypes.Structure):
    _fields_ = [("szName", ctypes.c_wchar_p), ("FmtValue", PDH_VALUE)]


class GpuMemory:
    """The process's dedicated GPU memory (bytes), from Windows' "GPU Process Memory" counters:
    the sum over the instances named `pid_<pid>_...`. None when the counters can't be read."""

    def __init__(self, pid):
        self.prefix, self.query, self.counter = f"pid_{pid}_", wintypes.HANDLE(), wintypes.HANDLE()
        ok = pdh.PdhOpenQueryW(None, 0, ctypes.byref(self.query)) == 0
        ok = ok and pdh.PdhAddEnglishCounterW(self.query, "\\GPU Process Memory(*)\\Dedicated Usage", 0,
                                               ctypes.byref(self.counter)) == 0
        self.ok = ok

    def read(self):
        if not self.ok or pdh.PdhCollectQueryData(self.query) != 0:
            return None
        size, count = wintypes.DWORD(0), wintypes.DWORD(0)
        r = pdh.PdhGetFormattedCounterArrayW(self.counter, PDH_FMT_LARGE, ctypes.byref(size), ctypes.byref(count), None)
        if r != PDH_MORE_DATA:
            return None
        buf = ctypes.create_string_buffer(size.value)
        if pdh.PdhGetFormattedCounterArrayW(self.counter, PDH_FMT_LARGE, ctypes.byref(size), ctypes.byref(count), buf) != 0:
            return None
        items = ctypes.cast(buf, ctypes.POINTER(PDH_ITEM))
        return sum(items[i].FmtValue.largeValue for i in range(count.value) if items[i].szName.startswith(self.prefix))

    def close(self):
        if self.ok:
            pdh.PdhCloseQuery(self.query)


def ft(f):
    return (f.dwHighDateTime << 32 | f.dwLowDateTime) / 1e7


def system_times():
    """(idle, busy) seconds of all cores since boot."""
    idle, kern, user = wintypes.FILETIME(), wintypes.FILETIME(), wintypes.FILETIME()
    kernel32.GetSystemTimes(ctypes.byref(idle), ctypes.byref(kern), ctypes.byref(user))
    return ft(idle), ft(kern) + ft(user) - ft(idle)


def process_cpu(handle):
    c, e, k, u = (wintypes.FILETIME() for _ in range(4))
    kernel32.GetProcessTimes(handle, ctypes.byref(c), ctypes.byref(e), ctypes.byref(k), ctypes.byref(u))
    return ft(k) + ft(u)


def load_percent(seconds=2.0):
    """The whole machine's CPU use over `seconds`, in percent of all cores."""
    i0, b0 = system_times()
    time.sleep(seconds)
    i1, b1 = system_times()
    return 100.0 * (b1 - b0) / max((b1 - b0) + (i1 - i0), 1e-9)


def memory(handle):
    pmc = PROCESS_MEMORY_COUNTERS_EX()
    pmc.cb = ctypes.sizeof(pmc)
    if not psapi.GetProcessMemoryInfo(handle, ctypes.byref(pmc), pmc.cb):
        return None
    return pmc.WorkingSetSize, pmc.PrivateUsage


def check(cmd):
    r = subprocess.run(cmd, capture_output=True, text=True)
    if r.returncode != 0:
        sys.exit(f"{' '.join(cmd)} failed:\n{r.stdout}{r.stderr}")


def build():
    os.makedirs(OUT, exist_ok=True)
    check([KESTREL, "build", os.path.join(VOXEL, "bench.kes"), "-o", os.path.join(OUT, "bench.exe"), "--release"])
    c100 = os.path.join(OUT, "c100")
    os.makedirs(c100, exist_ok=True)
    for f in os.listdir(c100):
        if f.endswith(".kes"):
            os.remove(os.path.join(c100, f))
    for f in os.listdir(VOXEL):
        if f.endswith(".kes"):
            shutil.copy(os.path.join(VOXEL, f), c100)
    path = os.path.join(c100, "bench.kes")
    text = open(path, encoding="utf-8").read()
    old = "let CIRCLE_SECONDS = 300"
    assert old in text, f"`{old}` is not in bench.kes"
    open(path, "w", encoding="utf-8", newline="\n").write(text.replace(old, "let CIRCLE_SECONDS = 100"))
    check([KESTREL, "build", path, "-o", os.path.join(OUT, "bench_c100.exe"), "--release"])


def run(exe, log):
    """One run of `exe` in a hidden window: its lines with times, the memory samples, the load."""
    before = load_percent()
    env = dict(os.environ, KESTREL_HIDDEN="1", KESTREL_ARENA_REPORT="1")
    for k in ("KESTREL_HEADLESS", "KESTREL_HEADLESS_QUIET", "KESTREL_SMOKE_VISIBLE", "KESTREL_POISON", "KESTREL_THREADS"):
        env.pop(k, None)
    s0 = system_times()
    t0 = time.perf_counter()
    p = subprocess.Popen([exe], stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True, env=env,
                         creationflags=IDLE_PRIORITY_CLASS | CREATE_NO_WINDOW)
    handle = kernel32.OpenProcess(PROCESS_QUERY_INFORMATION | PROCESS_VM_READ, False, p.pid)
    samples, lines, done = [], [], threading.Event()

    def sampler():
        gpu = GpuMemory(p.pid)
        while not done.is_set():
            m = memory(handle)
            if m:
                samples.append((time.perf_counter() - t0, m[0], m[1], gpu.read()))
            done.wait(SAMPLE_SECONDS)
        gpu.close()

    th = threading.Thread(target=sampler, daemon=True)
    th.start()
    for line in p.stdout:
        lines.append((time.perf_counter() - t0, line.rstrip("\n")))
    err = p.stderr.read()
    code = p.wait()
    cpu = process_cpu(handle)
    s1 = system_times()
    done.set()
    th.join()
    seconds = time.perf_counter() - t0
    total = (s1[1] - s0[1]) + (s1[0] - s0[0])
    others = 100.0 * max(s1[1] - s0[1] - cpu, 0.0) / max(total, 1e-9)
    with open(log, "w", encoding="utf-8") as f:
        f.writelines(f"{t:.3f} {l}\n" for t, l in lines)
        f.write(err)
    if code != 0 or not any(l == "done" for _, l in lines):
        sys.exit(f"{exe} exited {code} without `done`; see {log}")
    return parse(lines, samples, err, before, others, seconds)


def parse(lines, samples, err, before, others, seconds):
    phases, marks, r = {}, {}, {"load_before": before, "load_others_during": others, "seconds": seconds}
    for t, l in lines:
        w = l.split()
        if w[:1] == ["phase"]:
            phases[w[1]] = {k: float(w[w.index(k) + 1]) for k in PHASE_FIELDS}
            phases[w[1]]["at"] = t
        elif w[:1] == ["mark"]:
            marks[int(w[1])] = t
        elif w[:1] == ["edits"]:
            r["mid_ms"], r["corner_ms"] = float(w[2]), float(w[4])
        elif w[:1] == ["fill_ms"]:
            r["fill_ms"] = float(w[1])
        elif w[:1] == ["note:"]:
            r.setdefault("notes", []).append(l)
    m = re.search(r"program arena (\d+) bytes", err)
    r["arena"] = int(m.group(1)) if m else None
    r["phases"], r["marks"], r["samples"] = phases, marks, samples
    return r


def at(samples, t, k=1):
    """Field `k` (1 working set, 2 private bytes, 3 dedicated GPU memory) of the last sample taken
    at or before time `t` (the one after a line is printed may already see the exit's frees)."""
    before = [s for s in samples if s[0] <= t]
    return (before[-1] if before else samples[0])[k]


def curve(r):
    """The memory curve in bytes: the first sample, the fill's end, and every 50 s of the circle
    (working set, private bytes, dedicated GPU memory); and the E31 check over the circle's last
    200 s: the most the working set rose above its value at the start of that stretch."""
    s, marks = r["samples"], r["marks"]
    out = {}
    for name, k in (("ws", 1), ("private", 2), ("gpu", 3)):
        points = {"start": s[0][k], "after_fill": at(s, r["phases"]["fill"]["at"], k)}
        for sec in sorted(marks):
            if sec % 50 == 0:
                points[f"circle_{sec}"] = at(s, marks[sec], k)
        out[name] = points
    last = max(marks)
    if last >= 200:
        t_from, t_to = marks[last - 200], marks[last]
        window = [x[1] for x in s if t_from <= x[0] <= t_to]
        base = at(s, t_from)
        out["rise_last_200_percent"] = 100.0 * (max(window) - base) / base
        out["min_last_200"], out["max_last_200"] = min(window), max(window)
    return out


def show_curve(c):
    mb = 1024 * 1024
    parts = []
    for name in ("ws", "private", "gpu"):
        points = ", ".join(f"{k} {'?' if v is None else f'{v / mb:.1f}'}" for k, v in c[name].items())
        parts.append(f"  {name} (MB): {points}")
    if "rise_last_200_percent" in c:
        parts.append(f"  working set over the circle's last 200 s: min {c['min_last_200'] / mb:.1f}, max "
                     f"{c['max_last_200'] / mb:.1f} MB, rise {c['rise_last_200_percent']:.2f}%")
    return "\n".join(parts)


def spread(xs):
    xs = sorted(xs)
    return f"{statistics.median(xs):g} ({xs[0]:g} to {xs[-1]:g})"


def compile_times(rounds):
    """`--compile`: `kestrel build`, `build --release` and `check` of `main.kes`, in turn, `rounds`
    times each (idle priority); the median and range of each, in ms, and the load before."""
    main_kes = os.path.join(VOXEL, "main.kes")
    exe = os.path.join(OUT, "main_compile.exe")
    kinds = [("build", [KESTREL, "build", main_kes, "-o", exe]),
             ("build --release", [KESTREL, "build", main_kes, "-o", exe, "--release"]),
             ("check", [KESTREL, "check", main_kes])]
    print(f"load before: {load_percent():.1f}%", flush=True)
    times = {k: [] for k, _ in kinds}
    for _ in range(rounds):
        for k, cmd in kinds:
            t = time.perf_counter()
            r = subprocess.run(cmd, capture_output=True, creationflags=IDLE_PRIORITY_CLASS | CREATE_NO_WINDOW)
            if r.returncode != 0:
                sys.exit(f"{k} failed:\n{r.stdout.decode()}{r.stderr.decode()}")
            times[k].append((time.perf_counter() - t) * 1000.0)
    print(f"load after: {load_percent():.1f}%")
    for k, ts in times.items():
        print(f"{k:16} {spread([round(t) for t in ts])} ms")


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--runs", type=int, default=3)
    ap.add_argument("--skip-build", action="store_true")
    ap.add_argument("--no-arena", action="store_true")
    ap.add_argument("--compile", type=int, metavar="ROUNDS", help="time compiling main.kes instead (e.g. 9)")
    a = ap.parse_args()
    if a.compile:
        compile_times(a.compile)
        return
    if not a.skip_build:
        build()
    exe = os.path.join(OUT, "bench.exe")
    runs = []
    for n in range(a.runs):
        r = run(exe, os.path.join(OUT, f"measure_{n + 1}.txt"))
        runs.append(r)
        print(f"run {n + 1}: {r['seconds']:.1f} s, load before {r['load_before']:.1f}%, others during "
              f"{r['load_others_during']:.1f}%, arena {r['arena']}", flush=True)
    arena = {}
    if not a.no_arena:
        for name, path in (("c100", "bench_c100.exe"), ("c300", "bench.exe")):
            r = run(os.path.join(OUT, path), os.path.join(OUT, f"measure_arena_{name}.txt"))
            arena[name] = {"arena": r["arena"], "load_before": r["load_before"], "load_others_during": r["load_others_during"],
                           "curve": curve(r)}
            print(f"arena {name}: {r['arena']} bytes (load before {r['load_before']:.1f}%, others during "
                  f"{r['load_others_during']:.1f}%)", flush=True)

    print(f"\n{len(runs)} runs; each value is the median of the runs (range min to max)")
    for ph in ("fill", "fly20", "fly100", "edits", "circle"):
        print(f"phase {ph}:")
        for k in PHASE_FIELDS:
            print(f"  {k:10} {spread([r['phases'][ph][k] for r in runs])}")
    for k in ("fill_ms", "mid_ms", "corner_ms"):
        print(f"{k:10} {spread([r[k] for r in runs])}")
    for ph, secs in (("fly20", 20), ("fly100", 20), ("circle", 300)):
        print(f"columns per simulated second, {ph}: {spread([r['phases'][ph]['generated'] / secs for r in runs])}")
    print("E32 (work_p99 <= 16.7 ms):", {ph: max(r["phases"][ph]["work_p99"] for r in runs) <= 16.7
                                        for ph in ("fly20", "fly100", "circle")})
    for n, r in enumerate(runs):
        print(f"run {n + 1} memory:\n{show_curve(curve(r))}")
    if arena:
        print(f"arena c100 run memory:\n{show_curve(arena['c100']['curve'])}")
        print("E31 arena (bytes at exit):", arena["c100"]["arena"], "at 100 s,", arena["c300"]["arena"], "at 300 s:",
              "same" if arena["c100"]["arena"] == arena["c300"]["arena"] else "DIFFERENT")
    for n, r in enumerate(runs):
        for note in r.get("notes", []):
            print(f"run {n + 1}: {note}")
    keep = [{k: v for k, v in r.items() if k != "samples"} | {"curve": curve(r), "samples": r["samples"]} for r in runs]
    json.dump({"runs": keep, "arena": arena}, open(os.path.join(OUT, "measure.json"), "w"), indent=1)


if __name__ == "__main__":
    main()
