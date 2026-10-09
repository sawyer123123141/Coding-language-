fn main() {
    let n: usize = std::hint::black_box(50_000_000);
    let mut composite = vec![false; n];
    let mut count = 0;
    for i in 2..n {
        if !composite[i] {
            count += 1;
            let mut j = i * 2;
            while j < n {
                composite[j] = true;
                j += i;
            }
        }
    }
    println!("{}", count);
}
