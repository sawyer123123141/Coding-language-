#[derive(Clone, Copy)]
struct Particle {
    x: f32,
    y: f32,
    vx: f32,
    vy: f32,
}

impl Particle {
    fn step(&mut self, dt: f32) {
        self.vy -= 9.8 * dt;
        self.x += self.vx * dt;
        self.y += self.vy * dt;
        if self.y < 0.0 {
            self.y = -self.y;
            self.vy = -self.vy * 0.9;
        }
    }
}

fn main() {
    let n: i32 = std::hint::black_box(100_000_000);
    let dt: f32 = std::hint::black_box(0.001);
    let mut p = Particle { x: 0.0, y: 10.0, vx: 1.0, vy: 0.0 };
    let mut bounces = 0;
    for _ in 0..n {
        let before = p.vy;
        p.step(dt);
        if before < 0.0 && p.vy > 0.0 {
            bounces += 1;
        }
    }
    println!("{} {} {}", (p.x * 1000.0) as i32, (p.y * 1000.0) as i32, bounces);
}
