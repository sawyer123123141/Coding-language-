// Builds the 3D voxel world (8x4x8 chunks; the width is the argument) like
// main.kes. Width 0 measures startup.
int width = args.Length > 0 ? int.Parse(args[0]) : 8;
System.Console.WriteLine(Voxels3dCs.Bench(1337, width));
