#[derive(Clone, Copy)]
struct Particle {
    x: f32,
    y: f32,
    vx: f32,
    vy: f32,
}

fn main() {
    let n: i32 = std::hint::black_box(1_000_000);
    let mut ps = Vec::new();
    for i in 0..n {
        let f = (i % 1000) as f32;
        ps.push(Particle { x: f, y: f * 0.5, vx: 1.0 - f * 0.002, vy: 0.0 });
    }
    for _ in 0..100 {
        for p in ps.iter_mut() {
            p.vy -= 0.01;
            p.x += p.vx * 0.01;
            p.y += p.vy * 0.01;
            if p.y < 0.0 {
                p.y = -p.y;
                p.vy = -p.vy * 0.9;
            }
        }
    }
    let mut sum = 0.0f32;
    for p in &ps {
        sum += p.x + p.y;
    }
    println!("{}", sum as i32);
}
