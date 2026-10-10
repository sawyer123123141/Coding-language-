using System.Collections.Generic;
using System.Numerics;

/// The same voxel mesh generator as demo/voxels.kes, in C#: identical
/// arithmetic (float = 32-bit, hash in 64-bit ints), identical output.

public static class VoxelsCs
{
    const int Size = 32;
    const int HeightScale = 40;

    static int Hash(int x, int z, int seed)
    {
        long h = (long)x * 73856093 + (long)z * 19349663 + (long)seed * 83492791;
        return (int)(h % 1000003);
    }

    static float Lattice(int x, int z, int seed) => (float)Hash(x, z, seed) / 1000003.0f;

    static float Smooth(float t) => t * t * (3.0f - 2.0f * t);

    static float Noise(int x, int z, int scale, int seed)
    {
        int cx = x / scale;
        int cz = z / scale;
        float fx = Smooth((float)(x - cx * scale) / (float)scale);
        float fz = Smooth((float)(z - cz * scale) / (float)scale);
        float a = Lattice(cx, cz, seed);
        float b = Lattice(cx + 1, cz, seed);
        float c = Lattice(cx, cz + 1, seed);
        float d = Lattice(cx + 1, cz + 1, seed);
        float top = a + (b - a) * fx;
        float bottom = c + (d - c) * fx;
        return top + (bottom - top) * fz;
    }

    public static int HeightPub(int x, int z, int seed) => Height(x, z, seed);

    static int Height(int x, int z, int seed)
    {
        float n = Noise(x, z, 64, seed) * 0.6f + Noise(x, z, 16, seed + 1) * 0.3f + Noise(x, z, 4, seed + 2) * 0.1f;
        return (int)(n * (float)HeightScale) + 1;
    }

    static void Quad(List<Vector3> o, bool normals, Vector3 n, Vector3 a, Vector3 b, Vector3 c, Vector3 d)
    {
        if (normals)
        {
            for (int k = 0; k < 6; k++) o.Add(n);
        }
        else
        {
            o.Add(a); o.Add(b); o.Add(c);
            o.Add(a); o.Add(c); o.Add(d);
        }
    }

    static List<Vector3> Mesh(int cx, int cz, int seed, bool normals)
    {
        var o = new List<Vector3>();
        for (int i = 0; i < Size * Size; i++)
        {
            int x = cx * Size + i % Size;
            int z = cz * Size + i / Size;
            int h = Height(x, z, seed);
            float fx = x, fz = z, fh = h;
            Quad(o, normals, new Vector3(0, 1, 0), new Vector3(fx, fh, fz), new Vector3(fx + 1, fh, fz), new Vector3(fx + 1, fh, fz + 1), new Vector3(fx, fh, fz + 1));
            float east = Height(x + 1, z, seed);
            if (east < fh)
                Quad(o, normals, new Vector3(1, 0, 0), new Vector3(fx + 1, east, fz), new Vector3(fx + 1, east, fz + 1), new Vector3(fx + 1, fh, fz + 1), new Vector3(fx + 1, fh, fz));
            float west = Height(x - 1, z, seed);
            if (west < fh)
                Quad(o, normals, new Vector3(-1, 0, 0), new Vector3(fx, west, fz + 1), new Vector3(fx, west, fz), new Vector3(fx, fh, fz), new Vector3(fx, fh, fz + 1));
            float south = Height(x, z + 1, seed);
            if (south < fh)
                Quad(o, normals, new Vector3(0, 0, 1), new Vector3(fx + 1, south, fz + 1), new Vector3(fx, south, fz + 1), new Vector3(fx, fh, fz + 1), new Vector3(fx + 1, fh, fz + 1));
            float north = Height(x, z - 1, seed);
            if (north < fh)
                Quad(o, normals, new Vector3(0, 0, -1), new Vector3(fx, north, fz), new Vector3(fx + 1, north, fz), new Vector3(fx + 1, fh, fz), new Vector3(fx, fh, fz));
        }
        return o;
    }

    public static Vector3[] Vertices(int cx, int cz, int seed) => Mesh(cx, cz, seed, false).ToArray();

    public static Vector3[] Normals(int cx, int cz, int seed) => Mesh(cx, cz, seed, true).ToArray();

    /// Same as `bench` in voxels.kes.
    public static long Bench(int seed, int reps)
    {
        long total = 0;
        for (int r = 0; r < reps; r++)
            for (int cx = 0; cx < 4; cx++)
                for (int cz = 0; cz < 4; cz++)
                    total += Mesh(cx, cz, seed, false).Count + Mesh(cx, cz, seed, true).Count;
        return total;
    }
}
