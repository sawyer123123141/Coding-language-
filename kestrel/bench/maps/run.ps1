# Builds the map benchmark in Kestrel (checks on and off) and C#, checks the
# outputs agree, and prints each workload's median wall time with process
# startup subtracted. Run from kestrel/bench/maps in PowerShell.
param([int]$Runs = 7)
$ErrorActionPreference = "Stop"
$kestrel = Resolve-Path "..\..\target\release\kestrel.exe"

function Median([scriptblock]$cmd) {
    $times = 1..$Runs | ForEach-Object { (Measure-Command { & $cmd | Out-Null }).TotalMilliseconds }
    ($times | Sort-Object)[[int][math]::Floor($Runs / 2)]
}

function Check([string]$what) { if ($LASTEXITCODE -ne 0) { throw "$what failed" } }

foreach ($w in "int", "scattered", "words") {
    & $kestrel build "main_$w.kes" -o "k_$w.exe"; Check "kestrel build $w"
    & $kestrel build "main_$w.kes" -o "kr_$w.exe" --release; Check "kestrel build --release $w"
}
& $kestrel build main0.kes -o k_empty.exe; Check "kestrel build (empty)"
dotnet build cs -c Release -o cs\bin\out | Out-Null; Check "dotnet build"
$cs = ".\cs\bin\out\MapsBench.exe"

"| Workload | Kestrel (checks on) | Kestrel (``--release``) | C# .NET 9 |"
"|---|---:|---:|---:|"
$kStart = Median { & .\k_empty.exe }
$csStart = Median { & $cs none }
foreach ($w in "int", "scattered", "words") {
    $expected = (& $cs $w) -join ""
    foreach ($exe in ".\k_$w.exe", ".\kr_$w.exe") {
        $got = (& $exe) -join ""
        if ($got -ne $expected) { throw "$exe printed '$got', C# printed '$expected'" }
    }
    $on = (Median { & ".\k_$w.exe" }) - $kStart
    $off = (Median { & ".\kr_$w.exe" }) - $kStart
    $c = (Median { & $cs $w }) - $csStart
    "| $w | {0:N0} ms | {1:N0} ms | {2:N0} ms |" -f $on, $off, $c
}
"Startup: Kestrel {0:N0} ms, .NET {1:N0} ms" -f $kStart, $csStart
