fn collatz_len(start: i64) -> i32 {
    let mut n = start;
    let mut steps = 0;
    while n != 1 {
        if n % 2 == 0 { n /= 2 } else { n = 3 * n + 1 }
        steps += 1;
    }
    steps
}

fn main() {
    let limit: i64 = std::hint::black_box(1_000_000);
    let (mut best, mut best_start) = (0, 0i64);
    for s in 1..limit {
        let len = collatz_len(s);
        if len > best {
            best = len;
            best_start = s;
        }
    }
    println!("{best_start} {best}");
}
