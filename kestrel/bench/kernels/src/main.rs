//! Times a Kestrel kernel: compiles a `.kes` file the way `kestrel run` does, then calls its
//! exported `pub bench(n: int, iters: int) -> float` through the JIT, once with `iters` and once
//! with 0 (to take out the set-up), and prints the median time per item over `reps` pairs.
//!
//!   kernels <file.kes> [--release] [--n N] [--iters K] [--reps R] [--dump NAME]
//!
//! `--dump NAME` prints a function's optimised Cranelift IR and machine code instead (`par` for
//! every `par for` body). `KESTREL_THREADS` sets the `par for` thread count as usual.

use std::time::Instant;

use kestrel_codegen::{dump, ExportTy, JitProgram, Options};
use kestrel_driver::DiagStyle;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(path) = args.first().cloned() else {
        eprintln!("usage: kernels <file.kes> [--release] [--n N] [--iters K] [--reps R] [--dump NAME]");
        std::process::exit(2);
    };
    let flag = |name: &str| args.iter().position(|a| a == name).map(|i| args[i + 1].clone());
    let number = |name: &str, default: i64| flag(name).map_or(default, |v| v.parse().expect("a number"));
    let (n, iters, reps) = (number("--n", 200_000), number("--iters", 30), number("--reps", 9) as usize);
    let opts = Options { checked_arith: !args.iter().any(|a| a == "--release"), ..Options::default() };

    let src = std::fs::read_to_string(&path).expect("read the program");
    let (compiled, diagnostics) = kestrel_driver::compile(&path, &src, DiagStyle::Short);
    eprint!("{diagnostics}");
    let module = compiled.expect("the program compiles").module;
    if let Some(name) = flag("--dump") {
        let names: Vec<String> = if name == "par" {
            module.par_bodies.iter().map(|f| module.funcs[f.0 as usize].name.clone()).collect()
        } else {
            vec![name]
        };
        for name in names {
            println!("===== {name} =====\n{}", dump(&module, opts, &name).expect("dump"));
        }
        return;
    }

    let program = JitProgram::compile(&module, opts).expect("codegen");
    program.run_init();
    let (info, thunk) = program.exports().into_iter().find(|(i, _)| i.name == "bench").expect("a `pub bench(n: int, iters: int) -> float`");
    assert_eq!((info.params.as_slice(), info.ret.as_ref()), ([ExportTy::Int, ExportTy::Int].as_slice(), Some(&ExportTy::Float)));
    let call = |n: i64, k: i64| -> (f64, f32) {
        let mut buf = vec![0u8; info.args_size as usize];
        buf[info.arg_offsets[0] as usize..][..4].copy_from_slice(&(n as i32).to_le_bytes());
        buf[info.arg_offsets[1] as usize..][..4].copy_from_slice(&(k as i32).to_le_bytes());
        let mut ret = [0u8; 8];
        let start = Instant::now();
        // SAFETY: the buffer is laid out as `info` says, and `ret` holds the `float` result.
        unsafe { thunk(buf.as_ptr(), ret.as_mut_ptr()) };
        (start.elapsed().as_secs_f64(), f32::from_le_bytes(ret[..4].try_into().unwrap()))
    };
    call(n, 2);
    let mut per = Vec::new();
    let mut checksum = 0.0;
    for _ in 0..reps {
        let (base, _) = call(n, 0);
        let (t, sum) = call(n, iters);
        checksum = sum;
        per.push((t - base) / (n * iters) as f64 * 1e9);
    }
    per.sort_by(f64::total_cmp);
    println!(
        "{path}: {:.2} ns per item (median; {:.2}-{:.2}), n {n}, iters {iters}, reps {reps}, checksum {checksum}",
        per[per.len() / 2],
        per[0],
        per[per.len() - 1]
    );
}
