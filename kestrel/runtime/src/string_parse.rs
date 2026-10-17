//! `to_int` and `to_float`: whitespace around the number is ignored, then an optional sign and
//! digits (a float also takes a fraction and an exponent). Everything else is not a number:
//! `inf`, `nan`, hex, `_` separators, and values that don't fit.

use crate::string::KStr;

/// Whether `s` is `[+-]? digits`.
fn is_integer(s: &str) -> bool {
    let digits = s.strip_prefix(['+', '-']).unwrap_or(s);
    !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit())
}

/// Whether `s` is `[+-]? (digits [. digits?] | . digits) ([eE] [+-]? digits)?`.
fn is_decimal(s: &str) -> bool {
    let s = s.strip_prefix(['+', '-']).unwrap_or(s);
    let (mantissa, exponent) = match s.find(['e', 'E']) {
        Some(at) => (&s[..at], Some(&s[at + 1..])),
        None => (s, None),
    };
    let (whole, fraction) = match mantissa.split_once('.') {
        Some((w, f)) => (w, Some(f)),
        None => (mantissa, None),
    };
    let digits = |t: &str| t.bytes().all(|b| b.is_ascii_digit());
    let any_digit = !whole.is_empty() || fraction.is_some_and(|f| !f.is_empty());
    any_digit && digits(whole) && fraction.is_none_or(digits) && exponent.is_none_or(is_integer)
}

/// The text of `bytes`, trimmed, or `None` for text that isn't valid UTF-8.
fn trimmed(bytes: &[u8]) -> Option<&str> {
    std::str::from_utf8(bytes).ok().map(str::trim)
}

/// Parses an `int`. Returns 1 and writes `*out`, or returns 0 and leaves `*out` alone.
///
/// # Safety
/// The string must be valid for reading its bytes unless the pointer is null; `out` must be
/// valid for writing 4 bytes (not necessarily aligned).
#[no_mangle]
pub unsafe extern "C" fn kestrel_rt_str_to_int(ptr: *const u8, len: i64, out: *mut i32) -> i8 {
    // SAFETY: the caller guarantees the string.
    let bytes = unsafe { KStr::bytes(ptr, len) };
    let Some(text) = trimmed(bytes).filter(|t| is_integer(t)) else { return 0 };
    match text.parse::<i32>() {
        Ok(n) => {
            // SAFETY: the caller guarantees `out`.
            unsafe { out.write_unaligned(n) };
            1
        }
        Err(_) => 0,
    }
}

/// Parses a `float`. Returns 1 and writes `*out`, or returns 0 and leaves `*out` alone. A value
/// too large for a float is not a number.
///
/// # Safety
/// As `kestrel_rt_str_to_int`, with `out` valid for 4 bytes.
#[no_mangle]
pub unsafe extern "C" fn kestrel_rt_str_to_float(ptr: *const u8, len: i64, out: *mut f32) -> i8 {
    // SAFETY: the caller guarantees the string.
    let bytes = unsafe { KStr::bytes(ptr, len) };
    let Some(text) = trimmed(bytes).filter(|t| is_decimal(t)) else { return 0 };
    match text.parse::<f32>() {
        Ok(x) if x.is_finite() => {
            // SAFETY: the caller guarantees `out`.
            unsafe { out.write_unaligned(x) };
            1
        }
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn int(s: &str) -> Option<i32> {
        let mut out = 0;
        // SAFETY: valid string, `out` is an `i32`.
        (unsafe { kestrel_rt_str_to_int(s.as_ptr(), s.len() as i64, &mut out) } == 1).then_some(out)
    }

    fn float(s: &str) -> Option<f32> {
        let mut out = 0.0;
        // SAFETY: valid string, `out` is an `f32`.
        (unsafe { kestrel_rt_str_to_float(s.as_ptr(), s.len() as i64, &mut out) } == 1).then_some(out)
    }

    #[test]
    fn integers() {
        assert_eq!(int("42"), Some(42));
        assert_eq!(int(" -7 "), Some(-7));
        assert_eq!(int("+5"), Some(5));
        assert_eq!(int("\u{a0}12\u{2003}"), Some(12));
        assert_eq!(int("007"), Some(7));
        assert_eq!(int("2147483647"), Some(i32::MAX));
        assert_eq!(int("-2147483648"), Some(i32::MIN));
    }

    #[test]
    fn not_integers() {
        for s in ["", " ", "4x", "x4", "1_000", "0x10", "1.5", "+", "-", "--1", "2147483648", "-2147483649", "99999999999999999999", "inf", "nan", "٣"] {
            assert_eq!(int(s), None, "{s:?}");
        }
        let mut out = 0;
        // SAFETY: a null pointer is the empty string.
        assert_eq!(unsafe { kestrel_rt_str_to_int(std::ptr::null(), 0, &mut out) }, 0);
    }

    #[test]
    fn floats() {
        assert_eq!(float("2.5"), Some(2.5));
        assert_eq!(float(" -0.25 "), Some(-0.25));
        assert_eq!(float("3"), Some(3.0));
        assert_eq!(float("1e3"), Some(1000.0));
        assert_eq!(float("1.5E-1"), Some(0.15));
        assert_eq!(float(".5"), Some(0.5));
        assert_eq!(float("5."), Some(5.0));
        assert_eq!(float("+2"), Some(2.0));
    }

    #[test]
    fn not_floats() {
        for s in [
            "", ".", "e5", "1e", "1e+", "1.2.3", "inf", "-inf", "nan", "NaN", "infinity", "1e999", "-1e999", "0x1p3", "1_0.5", "1,5", "1e5.5", "+-1", "1 2",
        ] {
            assert_eq!(float(s), None, "{s:?}");
        }
    }
}
