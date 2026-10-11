use std::sync::atomic::{AtomicI32, Ordering};

pub const PANIC_OVERFLOW: i32 = 1;
pub const PANIC_DIV_ZERO: i32 = 2;
/// A narrowing conversion (`int(x)`, `u8(x)`) got a value the target can't hold.
pub const PANIC_NARROW_INT: i32 = 3;
pub const PANIC_NARROW_U8: i32 = 4;
/// `x << n` / `x >> n` with `n` negative or at least the type's width.
pub const PANIC_SHIFT: i32 = 5;
/// A loop over a map found its entry gone: the body replaced the map.
pub const PANIC_MAP_CHANGED: i32 = 6;

static FAILURE_HOOK: std::sync::OnceLock<fn(&str) -> !> = std::sync::OnceLock::new();

/// Makes runtime failures (overflow, bad index, ...) call `hook` instead of
/// ending the process: for hosts that embed Kestrel, like the Godot add-on.
/// `hook` must not return. The first hook set stays.
pub fn set_failure_hook(hook: fn(&str) -> !) {
    let _ = FAILURE_HOOK.set(hook);
}

/// Flushes output, then hands `msg` to the failure hook, or reports it and
/// exits with code 101.
pub fn fail(msg: &str) -> ! {
    // On a `par for` pool thread this hands the message to the caller instead.
    crate::par::fail_in_job(msg);
    crate::print::flush();
    if let Some(hook) = FAILURE_HOOK.get() {
        hook(msg);
    }
    eprintln!("kestrel: {msg}");
    std::process::exit(101);
}

/// The line of the latest call from a program's own code to a built-in function
/// that isn't `pure` (0 before the first). Compiled code stores it just before such
/// a call, so a native's error can name the line. Only the window's thread runs
/// those calls.
#[no_mangle]
#[allow(non_upper_case_globals)]
pub static kestrel_rt_call_line: AtomicI32 = AtomicI32::new(0);

/// Fails with `msg` and the line of the call into the built-in function that is
/// failing: "draw.circle was called before window.open on line 12".
pub fn fail_at_call(msg: &str) -> ! {
    match kestrel_rt_call_line.load(Ordering::Relaxed) {
        0 => fail(msg),
        line => fail(&format!("{msg} on line {line}")),
    }
}

#[no_mangle]
pub extern "C" fn kestrel_rt_panic_index(index: i64, len: i64, line: i32) -> ! {
    fail(&format!("index {index} out of bounds for length {len} on line {line}"));
}

#[no_mangle]
pub extern "C" fn kestrel_rt_panic(kind: i32, line: i32) -> ! {
    let what = match kind {
        PANIC_OVERFLOW => "integer overflow",
        PANIC_DIV_ZERO => "division by zero",
        PANIC_NARROW_INT => "value doesn't fit in int",
        PANIC_NARROW_U8 => "value doesn't fit in u8",
        PANIC_SHIFT => "shift amount out of range",
        PANIC_MAP_CHANGED => "a map was replaced while looping over it",
        _ => "runtime error",
    };
    fail(&format!("{what} on line {line}"));
}

#[no_mangle]
pub extern "C" fn kestrel_rt_fmod_f32(a: f32, b: f32) -> f32 {
    a % b
}

#[no_mangle]
pub extern "C" fn kestrel_rt_fmod_f64(a: f64, b: f64) -> f64 {
    a % b
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fmod_matches_rust_remainder() {
        assert_eq!(kestrel_rt_fmod_f32(7.5, 2.0), 1.5);
        assert_eq!(kestrel_rt_fmod_f64(-7.5, 2.0), -1.5);
    }
}
