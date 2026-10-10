// Builds the Godot demo's 4x4 voxel world N times, like main.kes; or a probe.
var mode = args.Length > 1 ? args[1] : "bench";
int reps = args.Length > 0 ? int.Parse(args[0]) : 60;
long t = 0;
for (int r = 0; r < (mode == "bench" ? 1 : reps); r++)
    t += mode switch { "heights" => Probe.HeightsOnly(1337) * 2, "adds" => Probe.AddsOnly(), _ => VoxelsCs.Bench(1337, reps) };
System.Console.WriteLine(t);
