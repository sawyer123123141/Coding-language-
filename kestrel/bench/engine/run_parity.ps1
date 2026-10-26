# Parity of the library-built voxel world with the frozen benchmark (E19, E36, E42).
#
# Builds the frozen `main.kes`, `main_par.kes` and `main0.kes` (from bench/voxels3d, with the
# output in bench/engine/out, never in the frozen folder) and the five parity entry files
# (kestrel/voxel/parity_*.kes), each with checks on and with --release, checks that every
# program prints the face count, then runs every exe in turn for $Rounds rounds (so a busy
# spell on the machine hits every variant alike) and prints the median and range of each,
# with the startup of the matching empty program subtracted. It also samples the machine's
# CPU load before every round and prints it. Run from kestrel/bench/engine in PowerShell:
#
#     .\run_parity.ps1
#
# It lowers its own priority to idle (children inherit it), so a game running beside it wins.
param([int]$Rounds = 11)
$ErrorActionPreference = "Stop"
(Get-Process -Id $PID).PriorityClass = "Idle"

$kestrel = (Resolve-Path "..\..\target\release\kestrel.exe").Path
$frozen = (Resolve-Path "..\voxels3d").Path
$voxel = (Resolve-Path "..\..\voxel").Path
$out = Join-Path (Get-Location).Path "out"
New-Item -ItemType Directory -Force $out | Out-Null

function Check([string]$what) { if ($LASTEXITCODE -ne 0) { throw "$what failed" } }

# (name, source, output name). The source is built twice: checks on ("on_") and --release ("rel_").
$programs = @(
    @("frozen_main", "$frozen\main.kes"), @("frozen_par", "$frozen\main_par.kes"), @("frozen_empty", "$frozen\main0.kes"),
    @("parity_main", "$voxel\parity_main.kes"), @("parity_get", "$voxel\parity_get.kes"),
    @("parity_reuse", "$voxel\parity_reuse.kes"), @("parity_par", "$voxel\parity_par.kes"),
    @("parity_empty", "$voxel\parity_empty.kes")
)
foreach ($p in $programs) {
    & $kestrel build $p[1] -o "$out\on_$($p[0]).exe"; Check "build $($p[0])"
    & $kestrel build $p[1] -o "$out\rel_$($p[0]).exe" --release; Check "build --release $($p[0])"
}

# Each variant: label, exe stem, empty program's stem. The plain exes run with KESTREL_THREADS=1,
# and the two par programs also run with all threads ("all_" keys below).
$kinds = @(
    @("frozen main.kes", "frozen_main", "frozen_empty"),
    @("PARITY_ITEMS", "parity_main", "parity_empty"),
    @("PARITY_GET", "parity_get", "parity_empty"),
    @("PARITY_REUSE", "parity_reuse", "parity_empty"),
    @("frozen par for, 1 thread", "frozen_par", "frozen_empty"),
    @("parity par for, 1 thread", "parity_par", "parity_empty"),
    @("frozen par for, all threads", "frozen_par", "frozen_empty"),
    @("parity par for, all threads", "parity_par", "parity_empty")
)
$modes = @(@("checks on", "on_"), @("--release", "rel_"))

function Run([string]$exe, $threads) {
    $env:KESTREL_THREADS = $threads
    try {
        $t = (Measure-Command { $script:last = (& $exe) -join "" }).TotalMilliseconds
    } finally { $env:KESTREL_THREADS = $null }
    $t
}

# Every exe must print the face count (the empty ones print 0).
foreach ($m in $modes) {
    foreach ($p in $programs) {
        $got = (& "$out\$($m[1])$($p[0]).exe") -join ""
        $want = if ($p[0] -like "*empty") { "0" } else { "463749" }
        if ($got -ne $want) { throw "$($m[1])$($p[0]) printed '$got', expected '$want'" }
    }
}

# The runs: every exe (the empties too) once per round.
$exes = @{}
foreach ($m in $modes) { foreach ($p in $programs) { $exes["$($m[1])$($p[0])"] = @() } }
$load = @()
for ($r = 1; $r -le $Rounds; $r++) {
    $load += (Get-Counter '\Processor(_Total)\% Processor Time' -SampleInterval 1 -MaxSamples 1).CounterSamples[0].CookedValue
    foreach ($m in $modes) {
        foreach ($p in $programs) {
            $key = "$($m[1])$($p[0])"
            $exes[$key] += Run "$out\$key.exe" "1"
        }
    }
    # The all-threads runs of the two par programs, which the loop above ran with one thread.
    foreach ($m in $modes) {
        foreach ($p in @("frozen_par", "parity_par")) {
            $key = "all_$($m[1])$p"
            if (-not $exes.ContainsKey($key)) { $exes[$key] = @() }
            $exes[$key] += Run "$out\$($m[1])$p.exe" $null
        }
    }
}

function Median($xs) { $s = $xs | Sort-Object; $s[[int][math]::Floor($s.Count / 2)] }

"Faces: 463749; rounds: $Rounds; logical cores: $([Environment]::ProcessorCount)"
"CPU load sampled before each round (the machine without this run): mean {0:N0}%, max {1:N0}%" -f ($load | Measure-Object -Average).Average, ($load | Measure-Object -Maximum).Maximum
""
"| Variant | Mode | Whole run (median) | Startup | World (median) | World range (min to max) |"
"|---|---|---:|---:|---:|---:|"
foreach ($m in $modes) {
    foreach ($k in $kinds) {
        $label = $k[0]
        $all = $label -like "*all threads*"
        $key = if ($all) { "all_$($m[1])$($k[1])" } else { "$($m[1])$($k[1])" }
        # Startup is the matching empty program's median; it does not depend on the thread count.
        $startup = Median $exes["$($m[1])$($k[2])"]
        $xs = $exes[$key]
        $med = Median $xs
        $lo = ($xs | Measure-Object -Minimum).Minimum
        $hi = ($xs | Measure-Object -Maximum).Maximum
        "| {0} | {1} | {2:N0} ms | {3:N0} ms | {4:N0} ms | {5:N0} to {6:N0} ms |" -f $label, $m[0], $med, $startup, ($med - $startup), ($lo - $startup), ($hi - $startup)
    }
}
