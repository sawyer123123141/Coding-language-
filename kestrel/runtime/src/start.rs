/// Runs a compiled Kestrel `main`, then flushes output. Returns the exit code.
#[no_mangle]
pub extern "C" fn kestrel_rt_start(entry: extern "C" fn()) -> i32 {
    entry();
    crate::print::flush();
    0
}

#[cfg(test)]
mod tests {
    use super::kestrel_rt_start;
    use std::sync::atomic::{AtomicBool, Ordering};

    static RAN: AtomicBool = AtomicBool::new(false);

    extern "C" fn entry() {
        RAN.store(true, Ordering::SeqCst);
    }

    #[test]
    fn start_runs_entry_and_returns_zero() {
        assert_eq!(kestrel_rt_start(entry), 0);
        assert!(RAN.load(Ordering::SeqCst));
    }
}
