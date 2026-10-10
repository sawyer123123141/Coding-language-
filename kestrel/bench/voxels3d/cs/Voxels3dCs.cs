using System.Collections.Generic;
using System.Numerics;

/// The same 3D voxel world as voxels3d.kes, in C#: identical arithmetic
/// (float = 32-bit, hash in 64-bit ints), identical output.

public static class Voxels3dCs
{
    const int Size = 32;
    const int Pad = Size + 2;
    const int Layers = 4;
    const int Height = Layers * Size;

    static int Hash(int x, int y, int z, int seed)
    {
        long h = (long)x * 73856093 ^ (long)y * 19349663 ^ (long)z * 83492791 ^ (long)seed * 1610612741;
        h = h & 0x7FFFFFFF;
        h = ((h ^ (h >> 13)) * 0x5BD1E995) & 0x7FFFFFFF;
        return (int)(h ^ (h >> 15));
    }

    static float Lattice(int x, int y, int z, int seed) => (float)(Hash(x, y, z, seed) & 0xFFFF) / 65535.0f;

    static float Smooth(float t) => t * t * (3.0f - 2.0f * t);

    static float Mix(float a, float b, float t) => a + (b - a) * t;

    static float Octave(int x, int y, int z, int shift, int seed)
    {
        int mask = (1 << shift) - 1;
        float scale = (float)(1 << shift);
        int cx = x >> shift;
        int cy = y >> shift;
        int cz = z >> shift;
        float fx = Smooth((float)(x & mask) / scale);
        float fy = Smooth((float)(y & mask) / scale);
        float fz = Smooth((float)(z & mask) / scale);
        float x00 = Mix(Lattice(cx, cy, cz, seed), Lattice(cx + 1, cy, cz, seed), fx);
        float x10 = Mix(Lattice(cx, cy + 1, cz, seed), Lattice(cx + 1, cy + 1, cz, seed), fx);
        float x01 = Mix(Lattice(cx, cy, cz + 1, seed), Lattice(cx + 1, cy, cz + 1, seed), fx);
        float x11 = Mix(Lattice(cx, cy + 1, cz + 1, seed), Lattice(cx + 1, cy + 1, cz + 1, seed), fx);
        return Mix(Mix(x00, x10, fy), Mix(x01, x11, fy), fz);
    }

    static float Density(int x, int y, int z, int seed)
    {
        float n = Octave(x, y, z, 5, seed) * 0.5f + Octave(x, y, z, 4, seed + 1) * 0.3f
            + Octave(x, y, z, 3, seed + 2) * 0.2f;
        return n - 0.5f - (float)(y - Height / 2) / (float)(Height * 2);
    }

    static byte[] Solids(int cx, int cy, int cz, int seed)
    {
        var o = new byte[Pad * Pad * Pad];
        int i = 0;
        for (int y = 0; y < Pad; y++)
            for (int z = 0; z < Pad; z++)
                for (int x = 0; x < Pad; x++)
                {
                    if (Density(cx * Size + x - 1, cy * Size + y - 1, cz * Size + z - 1, seed) > 0.0f)
                        o[i] = 1;
                    i++;
                }
        return o;
    }

    public sealed class Mesh
    {
        public List<Vector3> Vertices = new();
        public List<Vector3> Normals = new();
        public List<int> Indices = new();

        public void Quad(Vector3 n, Vector3 a, Vector3 b, Vector3 c, Vector3 d)
        {
            int b0 = Vertices.Count;
            Vertices.Add(a); Vertices.Add(b); Vertices.Add(c); Vertices.Add(d);
            for (int k = 0; k < 4; k++) Normals.Add(n);
            Indices.Add(b0); Indices.Add(b0 + 1); Indices.Add(b0 + 2);
            Indices.Add(b0); Indices.Add(b0 + 2); Indices.Add(b0 + 3);
        }
    }

    public static Mesh BuildMesh(int cx, int cy, int cz, int seed)
    {
        var solid = Solids(cx, cy, cz, seed);
        var m = new Mesh();
        for (int y = 0; y < Size; y++)
            for (int z = 0; z < Size; z++)
                for (int x = 0; x < Size; x++)
                {
                    int i = ((y + 1) * Pad + z + 1) * Pad + x + 1;
                    if (solid[i] == 1)
                    {
                        var p = new Vector3(cx * Size + x, cy * Size + y, cz * Size + z);
                        if (solid[i + 1] == 0)
                            m.Quad(new Vector3(1, 0, 0), p + new Vector3(1, 0, 0), p + new Vector3(1, 0, 1), p + new Vector3(1, 1, 1), p + new Vector3(1, 1, 0));
                        if (solid[i - 1] == 0)
                            m.Quad(new Vector3(-1, 0, 0), p + new Vector3(0, 0, 1), p, p + new Vector3(0, 1, 0), p + new Vector3(0, 1, 1));
                        if (solid[i + Pad * Pad] == 0)
                            m.Quad(new Vector3(0, 1, 0), p + new Vector3(0, 1, 0), p + new Vector3(1, 1, 0), p + new Vector3(1, 1, 1), p + new Vector3(0, 1, 1));
                        if (solid[i - Pad * Pad] == 0)
                            m.Quad(new Vector3(0, -1, 0), p, p + new Vector3(0, 0, 1), p + new Vector3(1, 0, 1), p + new Vector3(1, 0, 0));
                        if (solid[i + Pad] == 0)
                            m.Quad(new Vector3(0, 0, 1), p + new Vector3(1, 0, 1), p + new Vector3(0, 0, 1), p + new Vector3(0, 1, 1), p + new Vector3(1, 1, 1));
                        if (solid[i - Pad] == 0)
                            m.Quad(new Vector3(0, 0, -1), p, p + new Vector3(1, 0, 0), p + new Vector3(1, 1, 0), p + new Vector3(0, 1, 0));
                    }
                }
        return m;
    }

    public static int ChunkFaces(int cx, int cy, int cz, int seed) => BuildMesh(cx, cy, cz, seed).Indices.Count / 6;

    /// Same as `bench` in voxels3d.kes.
    public static long Bench(int seed, int width)
    {
        long faces = 0;
        for (int cx = 0; cx < width; cx++)
            for (int cy = 0; cy < Layers; cy++)
                for (int cz = 0; cz < width; cz++)
                    faces += ChunkFaces(cx, cy, cz, seed);
        return faces;
    }

    /// Same as `par_bench` in parallel.kes: every chunk on any core.
    public static long ParBench(int seed, int width)
    {
        int n = width * Layers * width;
        var faces = new int[n];
        System.Threading.Tasks.Parallel.For(0, n, i =>
        {
            int cx = i / (Layers * width), cy = i / width % Layers, cz = i % width;
            faces[i] = ChunkFaces(cx, cy, cz, seed);
        });
        long total = 0;
        foreach (int f in faces) total += f;
        return total;
    }
}
