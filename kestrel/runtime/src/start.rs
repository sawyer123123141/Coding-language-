/// Stack for the Kestrel program, so deep recursion fits.
const STACK_SIZE: usize = 256 * 1024 * 1024;

/// Runs `entry` and flushes on the calling thread, since output is buffered per thread.
fn run_entry(entry: extern "C" fn()) {
    entry();
    crate::print::flush();
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
    use super::kestrel_rt_start;
    use std::sync::atomic::{AtomicBool, Ordering};

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
