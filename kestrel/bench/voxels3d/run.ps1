# Builds the 3D voxel world in Kestrel (checks on and off) and C#, checks the
# outputs agree, and prints the median wall time of each with process startup
# (an empty world) subtracted. Run from kestrel/bench/voxels3d in PowerShell.
# -Tuned also builds and times voxels3d_tuned.kes (results.md, "Tuned copy").
param([int]$Runs = 7, [switch]$Tuned)
$ErrorActionPreference = "Stop"
$kestrel = Resolve-Path "..\..\target\release\kestrel.exe"

function Median([scriptblock]$cmd) {
    $times = 1..$Runs | ForEach-Object { (Measure-Command { & $cmd | Out-Null }).TotalMilliseconds }
    ($times | Sort-Object)[[int][math]::Floor($Runs / 2)]
}

function Check([string]$what) { if ($LASTEXITCODE -ne 0) { throw "$what failed" } }

& $kestrel build main.kes -o k_voxels3d.exe; Check "kestrel build"
& $kestrel build main.kes -o kr_voxels3d.exe --release; Check "kestrel build --release"
& $kestrel build main_par.kes -o k_par.exe; Check "kestrel build (par)"
& $kestrel build main_par.kes -o kr_par.exe --release; Check "kestrel build --release (par)"
& $kestrel build main0.kes -o k_empty.exe; Check "kestrel build (empty world)"
dotnet build cs -c Release -o cs\bin\out | Out-Null; Check "dotnet build"
$cs = ".\cs\bin\out\Voxels3dBench.exe"

$variants = [ordered]@{
    "Kestrel exe (checks on)" = @({ & .\k_voxels3d.exe }, { & .\k_empty.exe })
    "Kestrel exe (--release, checks off)" = @({ & .\kr_voxels3d.exe }, { & .\k_empty.exe })
    "C# .NET 9" = @({ & $cs 8 }, { & $cs 0 })
    "Kestrel par for, 1 thread (checks on)" = @({ $env:KESTREL_THREADS = "1"; & .\k_par.exe; $env:KESTREL_THREADS = $null }, { & .\k_empty.exe })
    "Kestrel par for, 1 thread (--release)" = @({ $env:KESTREL_THREADS = "1"; & .\kr_par.exe; $env:KESTREL_THREADS = $null }, { & .\k_empty.exe })
    "Kestrel par for, all threads (checks on)" = @({ & .\k_par.exe }, { & .\k_empty.exe })
    "Kestrel par for, all threads (--release)" = @({ & .\kr_par.exe }, { & .\k_empty.exe })
    "C# .NET 9 Parallel.For" = @({ & $cs 8 par }, { & $cs 0 })
}
if ($Tuned) {
    & $kestrel build voxels3d_tuned.kes -o k_tuned.exe; Check "kestrel build (tuned)"
    & $kestrel build voxels3d_tuned.kes -o kr_tuned.exe --release; Check "kestrel build --release (tuned)"
    $variants["Kestrel tuned exe (checks on)"] = @({ & .\k_tuned.exe }, { & .\k_empty.exe })
    $variants["Kestrel tuned exe (--release)"] = @({ & .\kr_tuned.exe }, { & .\k_empty.exe })
}
$expected = (& $cs 8) -join ""
foreach ($name in $variants.Keys) {
    $got = (& $variants[$name][0]) -join ""
    if ($got -ne $expected) { throw "$name printed '$got', expected '$expected'" }
}
"Faces: $expected"
"| Variant | Whole run | Startup | World |"
"|---|---:|---:|---:|"
foreach ($name in $variants.Keys) {
    $whole = Median $variants[$name][0]
    $start = Median $variants[$name][1]
    "| $name | {0:N0} ms | {1:N0} ms | {2:N0} ms |" -f $whole, $start, ($whole - $start)
}
