#!/bin/bash
# Rebuild and time all 5 benchmark workloads. Run from the benchmarks/
# directory. Requires cc (mingw/gcc) on PATH and kestrelc/target/release/
# kestrelc.exe already built (--features native).
set -e

export PATH="/c/Users/sawye/AppData/Local/Microsoft/WinGet/Packages/BrechtSanders.WinLibs.POSIX.UCRT_Microsoft.Winget.Source_8wekyb3d8bbwe/mingw64/bin:$PATH"
export USERPROFILE="${USERPROFILE:-C:\Users\sawye}"
unset HOME
KESTRELC="../kestrelc/target/release/kestrelc.exe"
TIMEFORMAT='%R'

median5() {
    local bin="$1"
    local times=()
    for i in 1 2 3 4 5; do
        local t
        t=$( { time "$bin" >/dev/null; } 2>&1 )
        times+=("$t")
    done
    printf '%s\n' "${times[@]}" | sort -n | sed -n '3p'
}

for dir in integer-loop fib-recursive array-sum parallel-map bounds-heavy; do
    echo "=== $dir ==="
    cd "$dir"
    cc -O2 bench.c -o bench_c_o2
    cc -O3 -march=native bench.c -o bench_c_o3
    "$KESTRELC" bench.kes
    # warm run for kestrelc's profile-guided inlining/memoization, then
    # recompile so the warmed profile is actually reflected in codegen
    ./bench >/dev/null
    "$KESTRELC" bench.kes >/dev/null

    out_k=$(./bench)
    out_o2=$(./bench_c_o2)
    out_o3=$(./bench_c_o3)
    if [ "$out_k" != "$out_o2" ] || [ "$out_k" != "$out_o3" ]; then
        echo "MISMATCH: kestrel=$out_k c-o2=$out_o2 c-o3=$out_o3"
        cd ..
        continue
    fi

    t_k=$(median5 ./bench)
    t_o2=$(median5 ./bench_c_o2)
    t_o3=$(median5 ./bench_c_o3)
    echo "kestrel=${t_k}s  c-o2=${t_o2}s  c-o3=${t_o3}s  (output: $out_k)"
    cd ..
done
