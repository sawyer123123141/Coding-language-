"""Runs every kernel, Kestrel and Rust, for a few interleaved rounds and prints the median of the
rounds' medians, with their range, in ns per item.

    py measure.py [--rounds 3] [--threads 1]

Run from kestrel/bench/kernels after `cargo build --release` there. `--threads` sets
KESTREL_THREADS for the Kestrel kernels (their `par for`); the Rust kernels are one thread.
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
    ("Rust step", ["rust", "step"], 30),
    ("Rust pow", ["rust", "pow"], 15),
    ("Rust copy", ["rust", "copy"], 100),
]


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--rounds", type=int, default=3)
    ap.add_argument("--threads", type=int, default=1)
    a = ap.parse_args()
    env = dict(os.environ, KESTREL_THREADS=str(a.threads))
    results = {label: [] for label, _, _ in KERNELS}
    for _ in range(a.rounds):
        for label, cmd, iters in KERNELS:
            exe = os.path.join(BIN, cmd[0] + (".exe" if os.name == "nt" else ""))
            out = subprocess.run([exe, *cmd[1:], "--iters", str(iters)], cwd=HERE, env=env, capture_output=True, text=True)
            m = re.search(r"([\d.]+) ns per item", out.stdout)
            if not m:
                raise SystemExit(f"{label} failed:\n{out.stdout}{out.stderr}")
            results[label].append(float(m.group(1)))
    print(f"{a.threads} thread(s) for Kestrel, {a.rounds} rounds")
    for label, vals in results.items():
        print(f"{label:46s} {statistics.median(vals):6.2f} ns  ({min(vals):.2f}-{max(vals):.2f})")


if __name__ == "__main__":
    main()
