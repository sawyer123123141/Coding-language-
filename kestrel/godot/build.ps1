# Builds the Kestrel Godot add-on (release) and installs it into the demo.
# Run from kestrel/godot in PowerShell. Close Godot first: it locks the DLL.
$ErrorActionPreference = "Stop"
cargo build --release
Copy-Item target\release\kestrel_godot.dll demo\addons\kestrel\bin\ -Force
Write-Host "Installed demo\addons\kestrel\bin\kestrel_godot.dll"
