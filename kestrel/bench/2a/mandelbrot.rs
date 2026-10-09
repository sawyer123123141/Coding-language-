fn main() {
    let size: i32 = std::hint::black_box(2000);
    let max_iter = 200;
    let mut inside = 0;
    let mut total_iter: i64 = 0;
    for py in 0..size {
        for px in 0..size {
            let cx = px as f32 / size as f32 * 3.0 - 2.0;
            let cy = py as f32 / size as f32 * 3.0 - 1.5;
            let (mut x, mut y) = (0.0f32, 0.0f32);
            let mut i = 0;
            while i < max_iter && x * x + y * y <= 4.0 {
                let xt = x * x - y * y + cx;
                y = 2.0 * x * y + cy;
                x = xt;
                i += 1;
            }
            total_iter += i as i64;
            if i == max_iter { inside += 1; }
        }
    }
    println!("{inside} {total_iter}");
}
