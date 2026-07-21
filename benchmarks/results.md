# Kestrel vs. C benchmark results

Run on: Windows, mingw `cc` (WinLibs UCRT), 16 logical cores. Median of 5
timed runs per variant, native builds only. See `2026-07-20-benchmark-suite-design.md`
in `docs/superpowers/specs/` for methodology.

| Workload | Kestrel | C -O2 | C -O3 -march=native | Kestrel ÷ C-O3 |
|---|---|---|---|---|
| integer-loop | 0.659s | 0.447s | 0.446s | **1.48x slower** |
| fib-recursive | 0.025s | 0.072s | 0.068s | **2.7x faster*** |
| array-sum | 0.167s | 0.163s | 0.158s | **1.06x slower** (near parity) |
| parallel-map | 0.077s | 0.354s | 0.350s | **4.5x faster** |
| bounds-heavy | 0.637s | 0.557s | 0.548s | **1.16x slower** |

\* fib-recursive's win is from automatic memoization eliminating naive
recursion's redundant subcalls, not from better codegen — C's naive
recursion has no equivalent optimization available. Not an
apples-to-apples codegen comparison; recorded honestly, not excluded.

## Reading these

- **On raw scalar codegen** (integer-loop, array-sum, bounds-heavy):
  Cranelift lands within roughly 6-48% of C `-O3`, closest when the
  workload doesn't autovectorize well on either side (array-sum, whose
  modulus reduction likely blocks vectorization in both compilers) and
  furthest on tight integer-only arithmetic (integer-loop). This is a
  real, moderate gap — not the >100% blowout a "Cranelift can't do
  vectorization" story alone would predict, since none of these three
  workloads triggered heavy vectorization on the C side either. A
  workload specifically designed to trigger SIMD (contiguous float
  arrays, unconditional element-wise ops with no modulus) would be a
  fairer test of Cranelift's actual vectorization gap and is a natural
  next addition to this suite.
- **On the actual thesis** (parallel-map): a clean **4.5x** win over
  single-threaded C, using purity-proven auto-parallelism with zero
  threading code written by hand. This is the strongest, most honest
  "beats C" result in the suite — it's not a codegen-quality claim, it's
  a language-semantics one, and the numbers back it.
- **bounds-heavy** shows the real, current cost of Kestrel's safety net
  for the dominant real-world array-access pattern (loop-indexed, not
  literal-indexed) — about 16% overhead versus C's raw unchecked access.
  See the finding below: the `where`-clause proof system doesn't yet
  cover this pattern at all, so every loop-indexed access pays a real
  runtime check today.

## Two real bugs found while building this suite

Neither is a benchmark artifact — both are genuine, previously-unknown
kestrelc issues, found by hitting them directly while sizing workloads.

### 1. Automatic memoization has no cost-benefit check

Any eligible `pure fn` (single scalar parameter, not a `parallel_map`
callback) gets an unconditional memoization slot
(`kestrelc/src/codegen.rs:534`). If that function is called with a
different argument on every single call — e.g. `square(i)` inside a
loop over `i` — every call is a guaranteed cache miss, but still pays to
grow and insert into an ever-larger hash table
(`kestrelc_runtime.c`'s `kestrelc_memo_store`).

Measured impact: a 200,000,000-iteration loop calling such a function
used over 3GB of RAM and did not finish within several minutes (killed
manually). The equivalent loop with the function call inlined away
(no memoization involved) ran in 0.66s. A 20,000,000-iteration version
of the same pathological pattern hit 2.5GB of RAM within 2 seconds.

This is a real risk for any Kestrel program with a `pure fn` applied
over a large index or counter — a very natural, common pattern, not a
contrived one. Worth a follow-up: either a runtime eviction/cap
strategy for a memo table that's clearly not getting hits, or a
compile-time heuristic (e.g. don't memoize a function whose only
call site's argument is provably monotonic/derived from a loop
counter).

### 2. Array literals are stack-allocated — large ones crash

`kestrelc/src/codegen.rs:310`'s own comment confirms this is
deliberate: "Array literals are stack-allocated." A 500,000-element
`i64` array literal (4MB) reliably crashed with
`STATUS_STACK_OVERFLOW` (`0xC00000FD`) against Windows' default 1MB
thread stack — confirmed via direct exit-code inspection
(`$LASTEXITCODE` = -1073741571 in PowerShell), not inferred.

A 100,000-element array (800KB) ran without crashing but left very
little headroom. All workloads in this suite were rebuilt at
20,000 elements (160KB) to stay safely clear of this limit.

This is a serious, silent failure mode for real Kestrel programs: any
moderately large data literal (not even that large — a few hundred
thousand entries) will crash with no compiler warning at compile time
and a cryptic OS-level crash at runtime, not a clean Kestrel error
message. Worth a follow-up: either heap-allocate array literals above
some size threshold, or at minimum have `resolve.rs`/`codegen.rs`
detect a literal large enough to be a stack-overflow risk and reject it
with a clear compile error instead of an opaque runtime crash.

## Files

- `run.sh` — rebuilds and times all 5 workloads, verifies output
  matches across all three variants, prints median-of-5 results.
- `<workload>/bench.kes` + `bench.c` — the workload pair.
