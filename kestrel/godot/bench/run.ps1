# Builds the C# side and the Kestrel add-on, then runs a benchmark script in Godot.
# Usage (from kestrel/godot/bench): ./run.ps1 <path to Godot console exe> [bench.gd | bench3d.gd]
param([Parameter(Mandatory = $true)][string]$Godot, [string]$Script = "bench.gd")
$ErrorActionPreference = "Stop"
$nupkgs = Join-Path (Split-Path $Godot) "GodotSharp\Tools\nupkgs"
# Godot loads the Debug build, which disables JIT optimizations unless asked.
dotnet build -p:Optimize=true -p:RestoreAdditionalProjectSources="$nupkgs"
if ($LASTEXITCODE -ne 0) { throw "dotnet build failed" }
New-Item -ItemType Directory -Force addons\kestrel\bin | Out-Null
Copy-Item ..\target\release\kestrel_godot.dll addons\kestrel\bin\ -Force
& $Godot --headless --path . --import | Out-Null
& $Godot --headless --path . --script "res://$Script"
