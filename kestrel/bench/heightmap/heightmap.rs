// The Kestrel heightmap ported to plain Rust: the same arithmetic in the
// same order (f32 floats, i32 ints, i64 hashing), so the checksum matches.
const SIZE: i32 = 32;
const WORLD: i32 = 16;
const HEIGHT: i32 = 64;

#[derive(Clone, Copy)]
enum Block {
    Air,
    Dirt,
    Stone { hardness: u8 },
}

impl Block {
    fn weight(self) -> i32 {
        match self {
            Block::Air => 0,
            Block::Dirt => 1,
            Block::Stone { hardness } => 2 + hardness as i32,
        }
    }
}

fn hash(x: i32, z: i32, seed: i32) -> i32 {
    let h = x as i64 * 73856093 + z as i64 * 19349663 + seed as i64 * 83492791;
    (h % 1000003) as i32
}

fn lattice(x: i32, z: i32, seed: i32) -> f32 {
    hash(x, z, seed) as f32 / 1000003.0
}

fn smooth(t: f32) -> f32 {
    t * t * (3.0 - 2.0 * t)
}

fn noise(x: i32, z: i32, scale: i32, seed: i32) -> f32 {
    let cx = x / scale;
    let cz = z / scale;
    let fx = smooth((x - cx * scale) as f32 / scale as f32);
    let fz = smooth((z - cz * scale) as f32 / scale as f32);
    let a = lattice(cx, cz, seed);
    let b = lattice(cx + 1, cz, seed);
    let c = lattice(cx, cz + 1, seed);
    let d = lattice(cx + 1, cz + 1, seed);
    let top = a + (b - a) * fx;
    let bottom = c + (d - c) * fx;
    top + (bottom - top) * fz
}

fn height(x: i32, z: i32, seed: i32) -> i32 {
    let n = noise(x, z, 64, seed) * 0.6 + noise(x, z, 16, seed + 1) * 0.3 + noise(x, z, 4, seed + 2) * 0.1;
    (n * (HEIGHT - 8) as f32) as i32 + 4
}

fn block_at(y: i32, ground: i32) -> Block {
    if y > ground {
        Block::Air
    } else if y > ground - 3 {
        Block::Dirt
    } else {
        Block::Stone { hardness: (3 + y % 4) as u8 }
    }
}

struct Chunk {
    x: i32,
    z: i32,
    heights: Vec<i32>,
}

impl Chunk {
    fn generate(&mut self, seed: i32) {
        self.heights = vec![0; (SIZE * SIZE) as usize];
        for i in 0..SIZE * SIZE {
            let wx = self.x * SIZE + i % SIZE;
            let wz = self.z * SIZE + i / SIZE;
            self.heights[i as usize] = height(wx, wz, seed);
        }
    }

    fn checksum(&self) -> i64 {
        let mut sum: i64 = 0;
        for (i, &h) in self.heights.iter().enumerate() {
            for y in 0..HEIGHT {
                sum += (block_at(y, h).weight() * (i as i32 % 7 + 1)) as i64;
            }
        }
        sum
    }
}

fn main() {
    let seed = std::hint::black_box(1337);
    let mut world = Vec::new();
    for cx in 0..WORLD {
        for cz in 0..WORLD {
            world.push(Chunk { x: cx, z: cz, heights: Vec::new() });
        }
    }
    for chunk in world.iter_mut() {
        chunk.generate(seed);
    }
    let mut total: i64 = 0;
    let mut tallest = 0;
    let mut columns = Vec::new();
    for chunk in &world {
        total += chunk.checksum();
        columns.extend_from_slice(&chunk.heights);
    }
    for &h in &columns {
        if h > tallest {
            tallest = h;
        }
    }
    println!("columns: {}", columns.len());
    println!("tallest: {tallest}");
    println!("checksum: {total}");
}
