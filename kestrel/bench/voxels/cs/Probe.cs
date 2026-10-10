using System.Collections.Generic;
using System.Numerics;

public static class Probe
{
    public static long HeightsOnly(int seed)
    {
        long total = 0;
        for (int cx = 0; cx < 4; cx++)
            for (int cz = 0; cz < 4; cz++)
                for (int i = 0; i < 32 * 32; i++)
                {
                    int x = cx * 32 + i % 32, z = cz * 32 + i / 32;
                    total += VoxelsCs.HeightPub(x, z, seed) + VoxelsCs.HeightPub(x + 1, z, seed) + VoxelsCs.HeightPub(x - 1, z, seed)
                        + VoxelsCs.HeightPub(x, z + 1, seed) + VoxelsCs.HeightPub(x, z - 1, seed);
                }
        return total;
    }

    public static long AddsOnly()
    {
        long total = 0;
        for (int r = 0; r < 32; r++)
        {
            var o = new List<Vector3>();
            for (int i = 0; i < 11094; i++) { float f = i; o.Add(new Vector3(f, f + 1, f)); }
            total += o.Count;
        }
        return total;
    }
}
