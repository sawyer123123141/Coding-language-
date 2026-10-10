# Builds the 3D voxel world in Kestrel (checks on and off) and C#, checks the
# outputs agree, and prints the median wall time of each with process startup
# (an empty world) subtracted. Run from kestrel/bench/voxels3d in PowerShell.
param([int]$Runs = 7)
$ErrorActionPreference = "Stop"
$kestrel = Resolve-Path "..\..\target\release\kestrel.exe"

function Median([scriptblock]$cmd) {
    $times = 1..$Runs | ForEach-Object { (Measure-Command { & $cmd | Out-Null }).TotalMilliseconds }
    ($times | Sort-Object)[[int][math]::Floor($Runs / 2)]
}

function Check([string]$what) { if ($LASTEXITCODE -ne 0) { throw "$what failed" } }

& $kestrel build main.kes -o k_voxels3d.exe; Check "kestrel build"
& $kestrel build main.kes -o kr_voxels3d.exe --release; Check "kestrel build --release"
& $kestrel build main0.kes -o k_empty.exe; Check "kestrel build (empty world)"
dotnet build cs -c Release -o cs\bin\out | Out-Null; Check "dotnet build"
$cs = ".\cs\bin\out\Voxels3dBench.exe"

$variants = [ordered]@{
    "Kestrel exe (checks on)" = @({ & .\k_voxels3d.exe }, { & .\k_empty.exe })
    "Kestrel exe (--release, checks off)" = @({ & .\kr_voxels3d.exe }, { & .\k_empty.exe })
    "C# .NET 9" = @({ & $cs 8 }, { & $cs 0 })
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
