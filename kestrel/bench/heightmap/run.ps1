# Builds the heightmap in Kestrel and Rust, checks the outputs agree, and
# prints median-of-7 wall times. Run from kestrel/bench/heightmap in PowerShell.
$ErrorActionPreference = "Stop"
$kestrel = Resolve-Path "..\..\target\release\kestrel.exe"
$runs = 7

function Median([scriptblock]$cmd) {
    $times = 1..$runs | ForEach-Object { (Measure-Command { & $cmd | Out-Null }).TotalMilliseconds }
    ($times | Sort-Object)[[int][math]::Floor($runs / 2)]
}

function Out-Of([scriptblock]$cmd) { ((& $cmd) -join "`n").Trim() }

& $kestrel build heightmap.kes -o k_heightmap.exe
& $kestrel build heightmap.kes -o kr_heightmap.exe --release
rustc -C opt-level=3 heightmap.rs -o rs_heightmap.exe 2>$null
rustc -C opt-level=3 -C overflow-checks=on heightmap.rs -o rsc_heightmap.exe 2>$null

$variants = [ordered]@{
    "Kestrel exe (checked)" = { & .\k_heightmap.exe }
    "Kestrel exe (--release)" = { & .\kr_heightmap.exe }
    "kestrel run (JIT, incl. compile)" = { & $kestrel run heightmap.kes }
    "kestrel check (front end only)" = { & $kestrel check heightmap.kes }
    "Rust -O3 (overflow checks on)" = { & .\rsc_heightmap.exe }
    "Rust -O3 (release default)" = { & .\rs_heightmap.exe }
}
$expected = Out-Of $variants["Rust -O3 (release default)"]
foreach ($name in $variants.Keys) {
    if ($name -like "kestrel check*") { continue }
    $got = Out-Of $variants[$name]
    if ($got -ne $expected) { throw "$name printed '$got', expected '$expected'" }
}
"| Variant | Time |"
"|---|---:|"
foreach ($name in $variants.Keys) { "| $name | {0:N0} ms |" -f (Median $variants[$name]) }
""
"Output: $($expected -replace "`n", ' / ')"
