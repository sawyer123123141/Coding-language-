/// Stack for the Kestrel program, so deep recursion fits.
const STACK_SIZE: usize = 256 * 1024 * 1024;

/// Runs `entry` and flushes on the calling thread, since output is buffered per thread.
fn run_entry(entry: extern "C" fn()) {
    entry();
    crate::print::flush();
    // The program arena belongs to this thread, so it is measured here.
    if let Some(report) = arena_report(std::env::var("KESTREL_ARENA_REPORT").ok().as_deref(), crate::arena::program_bytes()) {
        eprintln!("{report}");
    }
}

/// The line `KESTREL_ARENA_REPORT=1` asks for once `main` has returned. Tests use it to
/// tell a program that frees its temporary memory from one that doesn't.
fn arena_report(setting: Option<&str>, bytes: usize) -> Option<String> {
    (setting == Some("1")).then(|| format!("kestrel: program arena {bytes} bytes"))
}

/// Runs a compiled Kestrel `main` on a large stack, then flushes output.
/// Returns the exit code.
#[no_mangle]
pub extern "C" fn kestrel_rt_start(entry: extern "C" fn()) -> i32 {
    match std::thread::Builder::new().stack_size(STACK_SIZE).spawn(move || run_entry(entry)) {
        Ok(thread) => {
            // A panic in the runtime itself (not a Kestrel panic, which exits directly).
            if thread.join().is_ok() {
                0
            } else {
                101
            }
        }
        Err(_) => {
            // No room for a big stack: run on this thread rather than not at all.
            run_entry(entry);
            0
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{arena_report, kestrel_rt_start};
    use std::sync::atomic::{AtomicBool, Ordering};

    #[test]
    fn the_arena_report_is_on_only_for_one() {
        assert_eq!(arena_report(Some("1"), 65536).as_deref(), Some("kestrel: program arena 65536 bytes"));
        assert_eq!(arena_report(Some("0"), 1), None);
        assert_eq!(arena_report(Some("yes"), 1), None);
        assert_eq!(arena_report(None, 1), None);
    }

    static RAN: AtomicBool = AtomicBool::new(false);
    static DEEP: AtomicBool = AtomicBool::new(false);

    extern "C" fn entry() {
        RAN.store(true, Ordering::SeqCst);
    }

    #[test]
    fn start_runs_entry_and_returns_zero() {
        assert_eq!(kestrel_rt_start(entry), 0);
        assert!(RAN.load(Ordering::SeqCst));
    }

    /// Uses 1 MiB of stack per level.
    fn recurse(depth: u32) -> u8 {
        let block = [depth as u8; 1 << 20];
        let below = if depth == 0 { 0 } else { recurse(depth - 1) };
        std::hint::black_box(&block);
        below.wrapping_add(block[1 << 10])
    }

    extern "C" fn deep_entry() {
        // 64 MiB: far past a default thread stack, well inside ours.
        std::hint::black_box(recurse(64));
        DEEP.store(true, Ordering::SeqCst);
    }

    #[test]
    fn entry_runs_on_a_large_stack() {
        assert_eq!(kestrel_rt_start(deep_entry), 0);
        assert!(DEEP.load(Ordering::SeqCst));
    }
}
