// The same map benchmark as maps.kes, with Dictionary. Argument: "int",
// "scattered", "words" or "none" (startup only).
using System.Collections.Generic;

string mode = args.Length > 0 ? args[0] : "int";
System.Console.WriteLine(mode switch
{
    "int" => MapsCs.IntKeys(1_000_000, 5, false),
    "scattered" => MapsCs.IntKeys(1_000_000, 5, true),
    "words" => MapsCs.WordCount(10_000_000),
    _ => 0,
});

static class MapsCs
{
    static int Key(int i, int r, bool scattered) =>
        scattered ? (int)(((long)i * 2654435761L) & 0x3FFFFFFF) + r : i * 7 + r;

    public static long IntKeys(int n, int rounds, bool scattered)
    {
        long total = 0;
        for (int r = 0; r < rounds; r++)
        {
            var m = new Dictionary<int, int>();
            for (int i = 0; i < n; i++) m[Key(i, r, scattered)] = i;
            for (int i = 0; i < n; i++) total += m.GetValueOrDefault(Key(i, r, scattered), 0);
            for (int i = 0; i < n; i++) if (i % 2 == 0) m.Remove(Key(i, r, scattered));
            for (int i = 0; i < n; i++) if (m.ContainsKey(Key(i, r, scattered))) total += 1;
            total += m.Count;
        }
        return total;
    }

    static readonly string[] Words =
    {
        "sword", "bow", "axe", "shield", "potion", "arrow", "helmet", "boots",
        "ring", "amulet", "staff", "wand", "scroll", "gem", "coin", "key",
        "torch", "rope", "map", "lantern", "dagger", "spear", "mace", "hammer",
        "cloak", "gloves", "belt", "bread", "apple", "cheese", "water", "wine",
        "ore", "ingot", "plank", "stone", "brick", "glass", "cloth", "leather",
        "feather", "bone", "skull", "egg", "milk", "honey", "salt", "pepper",
        "iron", "gold", "silver", "copper", "tin", "lead", "ruby", "emerald",
        "sapphire", "diamond", "pearl", "amber", "jade", "onyx", "quartz", "topaz",
    };

    public static long WordCount(int ops)
    {
        // Insertion order, like Kestrel's maps, so the checksum matches.
        var counts = new Dictionary<string, int>();
        var order = new List<string>();
        long x = 12345;
        for (int i = 0; i < ops; i++)
        {
            x = (x * 1103515245 + 12345) & 0x7FFFFFFF;
            string w = Words[(int)(x >> 16) % Words.Length];
            if (!counts.TryGetValue(w, out int c)) order.Add(w);
            counts[w] = c + 1;
        }
        long total = counts.Count;
        foreach (var w in order) total = total * 31 % 1000000007 + counts[w];
        return total;
    }
}
