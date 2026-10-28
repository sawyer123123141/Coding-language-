//! The float control register. Kestrel's float rule is plain IEEE: round to nearest, every
//! exception masked, no flush-to-zero, no denormals-are-zero. `pin` sets that mode (MXCSR
//! `0x1F80`) on the calling thread; the runtime pins at the start of a program, when a thread
//! joins a `par for`, and (compiled code asks for it) after every call into the window library,
//! since a graphics driver may change the mode on the window's thread.

use std::cell::Cell;

/// MXCSR for round to nearest, all exceptions masked, no flush-to-zero, no denormals-are-zero.
pub const PINNED: u32 = 0x1F80;

/// What a program says on a CPU without SSE4.1 (exes and the JIT are compiled for SSE4.1).
pub const NO_SSE41: &str = "this program needs a CPU with SSE4.1 (every x86-64 CPU since about 2011); this one doesn't have it";

thread_local! {
    /// Whether this thread runs a program started by `kestrel_rt_start` (a host such as the Godot
    /// add-on keeps its own thread's float mode, so `par for` leaves its caller alone).
    static STARTED: Cell<bool> = const { Cell::new(false) };
}

/// Marks this thread as running a program started by `kestrel_rt_start`.
pub(crate) fn mark_started() {
    STARTED.with(|s| s.set(true));
}

/// Whether this thread was marked by `mark_started`.
pub(crate) fn started() -> bool {
    STARTED.with(Cell::get)
}

/// Whether this CPU has SSE4.1 (always true off x86-64, where nothing is compiled for it).
pub fn has_sse41() -> bool {
    #[cfg(target_arch = "x86_64")]
    {
        std::arch::is_x86_feature_detected!("sse4.1")
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        true
    }
}

/// Sets this thread's float mode to `PINNED`.
pub fn pin() {
    set(PINNED);
}

/// This thread's MXCSR (0 off x86-64).
pub fn read() -> u32 {
    #[cfg(target_arch = "x86_64")]
    {
        let mut value = 0u32;
        // SAFETY: `stmxcsr` stores the 32-bit register through a valid pointer.
        unsafe { core::arch::asm!("stmxcsr [{}]", in(reg) &mut value, options(nostack, preserves_flags)) };
        value
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        0
    }
}

/// Sets this thread's MXCSR (nothing off x86-64).
pub(crate) fn set(value: u32) {
    #[cfg(target_arch = "x86_64")]
    // SAFETY: `ldmxcsr` loads the register from a valid pointer; callers pass values that only
    // select rounding and masks.
    unsafe {
        core::arch::asm!("ldmxcsr [{}]", in(reg) &value, options(nostack, readonly, preserves_flags));
    }
    #[cfg(not(target_arch = "x86_64"))]
    let _ = value;
}

/// Compiled code calls this after every native of the window library returns.
#[no_mangle]
pub extern "C" fn kestrel_rt_fpu_pin() {
    pin();
}

#[cfg(all(test, target_arch = "x86_64"))]
mod tests {
    use super::*;
    use std::hint::black_box;

    #[test]
    fn pin_restores_round_to_nearest_without_flushing() {
        // Round toward zero, flush-to-zero, denormals-are-zero.
        set(0x7FC0);
        assert_eq!(read(), 0x7FC0);
        pin();
        assert_eq!(read(), 0x1F80);
        assert_ne!(black_box(f64::MIN_POSITIVE) * black_box(0.5), 0.0);
    }

    #[test]
    fn this_machine_has_sse41() {
        assert!(has_sse41());
    }
}
