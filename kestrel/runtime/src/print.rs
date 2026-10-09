use std::cell::RefCell;
use std::fmt::Arguments;
use std::io::{self, BufWriter, Stdout, Write};

thread_local! {
    static OUT: RefCell<BufWriter<Stdout>> = RefCell::new(BufWriter::new(io::stdout()));
}

fn write_fmt(args: Arguments) {
    OUT.with(|o| {
        let _ = o.borrow_mut().write_fmt(args);
    });
}

fn write_bytes(bytes: &[u8]) {
    OUT.with(|o| {
        let _ = o.borrow_mut().write_all(bytes);
    });
}

pub fn flush() {
    OUT.with(|o| {
        let _ = o.borrow_mut().flush();
    });
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
