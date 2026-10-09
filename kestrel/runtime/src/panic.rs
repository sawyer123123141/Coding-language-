pub const PANIC_OVERFLOW: i32 = 1;
pub const PANIC_DIV_ZERO: i32 = 2;
/// A narrowing conversion (`int(x)`, `u8(x)`) got a value the target can't hold.
pub const PANIC_NARROW_INT: i32 = 3;
pub const PANIC_NARROW_U8: i32 = 4;

/// Flushes output, reports `msg` and exits with code 101.
pub(crate) fn fail(msg: &str) -> ! {
    crate::print::flush();
    eprintln!("kestrel: {msg}");
    std::process::exit(101);
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
