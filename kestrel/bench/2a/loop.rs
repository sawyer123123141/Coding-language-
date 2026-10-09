fn main() {
    let n: i64 = std::hint::black_box(200_000_000);
    let mut total: i64 = 0;
    for i in 0..n {
        total = (total + i * i) % 1_000_000_007;
    }
    println!("{total}");
}
