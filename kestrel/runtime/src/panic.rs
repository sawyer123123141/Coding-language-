use std::sync::atomic::{AtomicI32, AtomicPtr, Ordering};

pub const PANIC_OVERFLOW: i32 = 1;
pub const PANIC_DIV_ZERO: i32 = 2;
/// A narrowing conversion (`int(x)`, `u8(x)`) got a value the target can't hold.
pub const PANIC_NARROW_INT: i32 = 3;
pub const PANIC_NARROW_U8: i32 = 4;
/// `x << n` / `x >> n` with `n` negative or at least the type's width.
pub const PANIC_SHIFT: i32 = 5;
/// A loop over a map found its entry gone: the body replaced the map.
pub const PANIC_MAP_CHANGED: i32 = 6;
/// A bodiless interface method was called (the compiler never emits such a call).
pub const PANIC_ABSTRACT: i32 = 7;

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

/// A source file's name as compiled code passes it to a runtime error: static data holding a
/// pointer to the name's bytes and its length. Compiled code passes null when it has no name.
#[repr(C)]
pub struct FileName {
    ptr: *const u8,
    len: i64,
}

/// " of geom.kes" for an error in that file, or nothing for null.
pub(crate) fn of_file(file: *const FileName) -> String {
    if file.is_null() {
        return String::new();
    }
    // SAFETY: compiled code passes a pointer to its static `FileName`, whose bytes are static too.
    let name = unsafe { std::slice::from_raw_parts((*file).ptr, (*file).len as usize) };
    format!(" of {}", String::from_utf8_lossy(name))
}

/// The file of the call whose line is in `kestrel_rt_call_line`, stored with it.
#[no_mangle]
#[allow(non_upper_case_globals)]
pub static kestrel_rt_call_file: AtomicPtr<FileName> = AtomicPtr::new(std::ptr::null_mut());

/// Fails with `msg` and the line of the call into the built-in function that is
/// failing: "draw.circle was called before window.open on line 12".
///
/// A native that a `pure` built-in function can reach must use plain `fail` instead: calls to a
/// `pure` function never store the line, so the one here would be stale (the line of some earlier
/// call), and the `pure` function may be running on a `par for` thread.
pub fn fail_at_call(msg: &str) -> ! {
    match kestrel_rt_call_line.load(Ordering::Relaxed) {
        0 => fail(msg),
        line => fail(&format!("{msg} on line {line}{}", of_file(kestrel_rt_call_file.load(Ordering::Relaxed)))),
    }
}

#[no_mangle]
pub extern "C" fn kestrel_rt_panic_index(index: i64, len: i64, line: i32, file: *const FileName) -> ! {
    fail(&format!("index {index} out of bounds for length {len} on line {line}{}", of_file(file)));
}

#[no_mangle]
pub extern "C" fn kestrel_rt_panic(kind: i32, line: i32, file: *const FileName) -> ! {
    let what = match kind {
        PANIC_OVERFLOW => "integer overflow",
        PANIC_DIV_ZERO => "division by zero",
        PANIC_NARROW_INT => "value doesn't fit in int",
        PANIC_NARROW_U8 => "value doesn't fit in u8",
        PANIC_SHIFT => "shift amount out of range",
        PANIC_MAP_CHANGED => "a map was replaced while looping over it",
        PANIC_ABSTRACT => "an interface method with no body was called",
        _ => "runtime error",
    };
    fail(&format!("{what} on line {line}{}", of_file(file)));
}

/// Fails with the message in the `len` bytes at `msg`: a compiled error message with no line (the
/// copy function of an owning type's method table).
///
/// # Safety
/// `msg` points at `len` bytes of UTF-8 that stay valid (compiled code passes static data).
#[no_mangle]
pub unsafe extern "C" fn kestrel_rt_fail(msg: *const u8, len: i64) -> ! {
    // SAFETY: the caller passes `len` readable bytes at `msg`.
    let bytes = unsafe { std::slice::from_raw_parts(msg, len as usize) };
    fail(&String::from_utf8_lossy(bytes));
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
