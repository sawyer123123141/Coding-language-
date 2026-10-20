"""Runs every kernel, Kestrel and Rust, for a few interleaved rounds and prints the median of the
rounds' medians, with their range, in ns per item.

    py measure.py [--rounds 3] [--threads 1] [--only TEXT]

Run from kestrel/bench/kernels after `cargo build --release` there. `--threads` sets
KESTREL_THREADS for the Kestrel kernels (their `par for`); the Rust kernels are one thread.
`--only TEXT` runs just the kernels whose label contains TEXT (case-insensitive), e.g. `--only iface`
for the interface kernels, Kestrel and Rust.
"""
import argparse, os, re, statistics, subprocess

HERE = os.path.dirname(os.path.abspath(__file__))
BIN = os.path.join(HERE, "target", "release")

# (label, command, iterations): more iterations for the cheap kernels.
KERNELS = [
    ("step.kes (the demo's step, pow hoisted)", ["kernels", "step.kes", "--release"], 30),
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
]


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--rounds", type=int, default=3)
    ap.add_argument("--threads", type=int, default=1)
    ap.add_argument("--only", default="")
    a = ap.parse_args()
    env = dict(os.environ, KESTREL_THREADS=str(a.threads))
    kernels = [k for k in KERNELS if a.only.lower() in k[0].lower()]
    results = {label: [] for label, _, _ in kernels}
    for _ in range(a.rounds):
        for label, cmd, iters in kernels:
            exe = os.path.join(BIN, cmd[0] + (".exe" if os.name == "nt" else ""))
            out = subprocess.run([exe, *cmd[1:], "--iters", str(iters)], cwd=HERE, env=env, capture_output=True, text=True)
            m = re.search(r"([\d.]+) ns per item", out.stdout)
            if not m:
                raise SystemExit(f"{label} failed:\n{out.stdout}{out.stderr}")
            results[label].append(float(m.group(1)))
    print(f"{a.threads} thread(s) for Kestrel, {a.rounds} rounds")
    for label, vals in results.items():
        print(f"{label:56s} {statistics.median(vals):6.2f} ns  ({min(vals):.2f}-{max(vals):.2f})")


if __name__ == "__main__":
    main()
