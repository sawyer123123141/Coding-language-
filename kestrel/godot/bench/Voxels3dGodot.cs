using Godot;

/// The C# 3D voxel world from bench/voxels3d/cs (linked into this project
/// by KestrelBench.csproj, so it is the same source), callable from GDScript.
[GlobalClass]
public partial class Voxels3dGodot : RefCounted
{
    public long Bench(int seed, int width) => Voxels3dCs.Bench(seed, width);

    public int ChunkFaces(int cx, int cy, int cz, int seed) => Voxels3dCs.ChunkFaces(cx, cy, cz, seed);

    public long ParBench(int seed, int width) => Voxels3dCs.ParBench(seed, width);
}
