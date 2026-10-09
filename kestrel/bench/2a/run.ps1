# Builds every workload in each language variant, checks all outputs agree,
# and prints a markdown table of median-of-5 wall times.
# Run from kestrel/bench/2a in PowerShell (Git Bash adds ~60 ms per process).
$ErrorActionPreference = "Stop"
$kestrel = Resolve-Path "..\..\target\release\kestrel.exe"
$workloads = "fib", "loop", "collatz", "mandelbrot"
$runs = 5

function Median([scriptblock]$cmd) {
    $times = 1..$runs | ForEach-Object { (Measure-Command { & $cmd | Out-Null }).TotalMilliseconds }
    ($times | Sort-Object)[[int][math]::Floor($runs / 2)]
}

function Out-Of([scriptblock]$cmd) { ((& $cmd) -join "`n").Trim() }

$rows = @()
foreach ($w in $workloads) {
    & $kestrel build "$w.kes" -o "k_$w.exe"
    & $kestrel build "$w.kes" -o "kr_$w.exe" --release
    rustc -C opt-level=3 "$w.rs" -o "rs_$w.exe" 2>$null
    rustc -C opt-level=3 -C overflow-checks=on "$w.rs" -o "rsc_$w.exe" 2>$null
    gcc -O2 "$w.c" -o "c_$w.exe"

    $variants = [ordered]@{
        "Kestrel exe (checked)"     = [scriptblock]::Create("& .\k_$w.exe")
        "Kestrel exe (--release)"   = [scriptblock]::Create("& .\kr_$w.exe")
        "kestrel run (JIT, incl. compile)" = [scriptblock]::Create("& '$kestrel' run $w.kes")
        "Rust -O3 (overflow checks on)" = [scriptblock]::Create("& .\rsc_$w.exe")
        "Rust -O3 (release default)" = [scriptblock]::Create("& .\rs_$w.exe")
        "C gcc -O2"                 = [scriptblock]::Create("& .\c_$w.exe")
    }

    $expected = Out-Of $variants["Rust -O3 (release default)"]
    foreach ($name in $variants.Keys) {
        $got = Out-Of $variants[$name]
        if ($got -ne $expected) { throw "$w / $name printed '$got', expected '$expected'" }
    }
    $times = [ordered]@{}
    foreach ($name in $variants.Keys) { $times[$name] = Median $variants[$name] }
    $rows += [pscustomobject]@{ Workload = $w; Output = $expected; Times = $times }
}

$names = $rows[0].Times.Keys
"| Workload | " + ($names -join " | ") + " |"
"|---|" + (($names | ForEach-Object { "---:" }) -join "|") + "|"
foreach ($r in $rows) {
    "| $($r.Workload) | " + (($names | ForEach-Object { "{0:N0} ms" -f $r.Times[$_] }) -join " | ") + " |"
}
""
foreach ($r in $rows) { "$($r.Workload) output: $($r.Output)" }
