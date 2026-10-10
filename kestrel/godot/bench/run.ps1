# Builds the C# side and the Kestrel add-on, then runs bench.gd in Godot.
# Usage (from kestrel/godot/bench): ./run.ps1 <path to Godot console exe>
param([Parameter(Mandatory = $true)][string]$Godot)
$ErrorActionPreference = "Stop"
$nupkgs = Join-Path (Split-Path $Godot) "GodotSharp\Tools\nupkgs"
dotnet build -c Release -p:RestoreAdditionalProjectSources="$nupkgs"
New-Item -ItemType Directory -Force addons\kestrel\bin | Out-Null
Copy-Item ..\target\release\kestrel_godot.dll addons\kestrel\bin\ -Force
& $Godot --headless --path . --import | Out-Null
& $Godot --headless --path . --script res://bench.gd
