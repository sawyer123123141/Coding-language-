// Builds the 3D voxel world (8x4x8 chunks; the width is the first argument)
// like main.kes, or with Parallel.For like main_par.kes when the second
// argument is "par". Width 0 measures startup.
int width = args.Length > 0 ? int.Parse(args[0]) : 8;
bool par = args.Length > 1 && args[1] == "par";
System.Console.WriteLine(par ? Voxels3dCs.ParBench(1337, width) : Voxels3dCs.Bench(1337, width));
