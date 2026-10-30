use std::cell::RefCell;
use std::fmt::Arguments;
use std::io::{self, BufWriter, Stdout, Write};

/// Where this thread's `print` output goes.
enum Sink {
    Stdout(BufWriter<Stdout>),
    /// Kept for the host to collect (see `capture_output`).
    Buffer(Vec<u8>),
}

impl Write for Sink {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        match self {
            Sink::Stdout(w) => {
                let n = w.write(bytes)?;
                // A recording keeps what reaches stdout; a replay compares it, and stops at a
                // line that differs, after showing the output that came before it.
                if let Err(stop) = crate::replay::print_bytes(&bytes[..n]) {
                    let _ = w.flush();
                    stop.exit();
                }
                Ok(n)
            }
            Sink::Buffer(b) => b.write(bytes),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match self {
            Sink::Stdout(w) => w.flush(),
            Sink::Buffer(_) => Ok(()),
        }
    }
}

thread_local! {
    static OUT: RefCell<Sink> = RefCell::new(Sink::Stdout(BufWriter::new(io::stdout())));
}

thread_local! {
    /// Open string buffers, innermost last. While there are any, `print`
    /// output builds a string (see `kestrel_rt_str_begin`) instead of going to `OUT`.
    static BUILD: RefCell<Vec<Vec<u8>>> = const { RefCell::new(Vec::new()) };
}

fn write_fmt(args: Arguments) {
    let built = BUILD.with_borrow_mut(|b| b.last_mut().map(|buf| buf.write_fmt(args)).is_some());
    if !built {
        OUT.with(|o| {
            let _ = o.borrow_mut().write_fmt(args);
        });
    }
}

fn write_bytes(bytes: &[u8]) {
    let built = BUILD.with_borrow_mut(|b| b.last_mut().map(|buf| buf.extend_from_slice(bytes)).is_some());
    if !built {
        OUT.with(|o| {
            let _ = o.borrow_mut().write_all(bytes);
        });
    }
}

pub fn flush() {
    OUT.with(|o| {
        let _ = o.borrow_mut().flush();
    });
}

/// From now on, this thread's `print` output is kept in memory instead of
/// written to stdout; `take_output` collects it.
pub fn capture_output() {
    flush();
    OUT.with(|o| *o.borrow_mut() = Sink::Buffer(Vec::new()));
}

/// The output captured on this thread since the last call (empty when not capturing).
pub fn take_output() -> Vec<u8> {
    OUT.with(|o| match &mut *o.borrow_mut() {
        Sink::Buffer(b) => std::mem::take(b),
        Sink::Stdout(_) => Vec::new(),
    })
}

#[no_mangle]
pub extern "C" fn kestrel_rt_print_i64(v: i64) {
    write_fmt(format_args!("{v}"));
}

#[no_mangle]
pub extern "C" fn kestrel_rt_print_f32(v: f32) {
    write_fmt(format_args!("{v}"));
}

#[no_mangle]
pub extern "C" fn kestrel_rt_print_f64(v: f64) {
    write_fmt(format_args!("{v}"));
}

#[no_mangle]
pub extern "C" fn kestrel_rt_print_bool(v: i8) {
    write_bytes(if v != 0 { b"true" } else { b"false" });
}

/// # Safety
/// `ptr` must point to `len` readable bytes (compiled code passes string data).
#[no_mangle]
pub unsafe extern "C" fn kestrel_rt_print_str(ptr: *const u8, len: i64) {
    // A zero-filled string (from `array[string](n)`) is null with length 0.
    if ptr.is_null() || len <= 0 {
        return;
    }
    write_bytes(std::slice::from_raw_parts(ptr, len as usize));
}

#[no_mangle]
pub extern "C" fn kestrel_rt_print_space() {
    write_bytes(b" ");
}

#[no_mangle]
pub extern "C" fn kestrel_rt_print_newline() {
    write_bytes(b"\n");
}

/// From now on, this thread's `print` output builds a string instead.
/// Buffers nest: `kestrel_rt_str_end` ends the innermost one.
#[no_mangle]
pub extern "C" fn kestrel_rt_str_begin() {
    BUILD.with_borrow_mut(|b| b.push(Vec::new()));
}

/// Ends the innermost string buffer: copies its bytes into the current arena
/// and writes the pointer at `out` and the length (an `i64`) at `out + 8`.
/// An empty buffer gives a null pointer and 0, as a zero-filled string is.
///
/// # Safety
/// `out` must be valid for writing 16 bytes, and a `kestrel_rt_str_begin`
/// must be open.
#[no_mangle]
pub unsafe extern "C" fn kestrel_rt_str_end(out: *mut u8) {
    let buf = BUILD.with_borrow_mut(|b| b.pop().expect("string buffers end where they begin"));
    let ptr = if buf.is_empty() {
        std::ptr::null_mut()
    } else {
        let p = crate::arena::alloc_unlimited(buf.len() as i64, 1, 1, 0, std::ptr::null());
        // SAFETY: `p` has room for `buf.len()` bytes and is fresh, so it can't overlap `buf`.
        unsafe { std::ptr::copy_nonoverlapping(buf.as_ptr(), p, buf.len()) };
        p
    };
    // SAFETY: the caller guarantees 16 writable bytes; compiled code's slot may not be 8-aligned.
    unsafe {
        out.cast::<*mut u8>().write_unaligned(ptr);
        out.add(8).cast::<i64>().write_unaligned(buf.len() as i64);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn captured_output_is_collected_per_call() {
        std::thread::spawn(|| {
            capture_output();
            kestrel_rt_print_i64(42);
            kestrel_rt_print_newline();
            assert_eq!(take_output(), b"42\n");
            assert_eq!(take_output(), b"");
        })
        .join()
        .unwrap();
    }

    /// Reads back a `(ptr, len)` pair that `kestrel_rt_str_end` wrote.
    fn read_string(out: &[u8; 16]) -> (*const u8, i64, Vec<u8>) {
        let ptr = usize::from_ne_bytes(out[..8].try_into().unwrap()) as *const u8;
        let len = i64::from_ne_bytes(out[8..].try_into().unwrap());
        let bytes = if len == 0 {
            Vec::new()
        } else {
            // SAFETY: `str_end` copied `len` bytes to `ptr` in the thread's arena.
            unsafe { std::slice::from_raw_parts(ptr, len as usize) }.to_vec()
        };
        (ptr, len, bytes)
    }

    #[test]
    fn strings_nest_and_land_in_the_arena() {
        std::thread::spawn(|| {
            capture_output();
            let mut outer = [0u8; 16];
            let mut inner = [0u8; 16];
            // SAFETY: each `out` has room for a pointer and a length.
            unsafe {
                kestrel_rt_str_begin();
                kestrel_rt_print_i64(4);
                kestrel_rt_str_begin();
                kestrel_rt_print_str(b"x".as_ptr(), 1);
                kestrel_rt_str_end(inner.as_mut_ptr());
                kestrel_rt_print_i64(2);
                kestrel_rt_str_end(outer.as_mut_ptr());
            }
            assert_eq!(read_string(&outer).2, b"42");
            assert_eq!(read_string(&inner).2, b"x");
            assert_eq!(take_output(), b"", "nothing leaked to the capture");

            // An empty buffer is a null pointer and length 0.
            let mut empty = [0xFFu8; 16];
            // SAFETY: `empty` has room for a pointer and a length.
            unsafe {
                kestrel_rt_str_begin();
                kestrel_rt_str_end(empty.as_mut_ptr());
            }
            let (ptr, len, _) = read_string(&empty);
            assert!(ptr.is_null());
            assert_eq!(len, 0);
        })
        .join()
        .unwrap();
    }
}
