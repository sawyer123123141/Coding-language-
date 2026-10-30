//! Why the last call of a native module failed: `file.error()`'s answer. Each thread has its own,
//! and natives run on the thread that calls them, so a program's main thread sees its own calls.
//!
//! Every call of a module that can fail sets it: `set` with a sentence on a failure, `clear` on a
//! success. Reading it with `get` leaves it in place, so `file.error()` gives the same answer until
//! the next call of the module.

use std::cell::RefCell;
use std::io;

thread_local! {
    static LAST: RefCell<String> = const { RefCell::new(String::new()) };
}

/// Records `msg` as why the last call failed.
pub fn set(msg: String) {
    LAST.with_borrow_mut(|last| *last = msg);
}

/// Records that the last call succeeded: the error is empty again.
pub fn clear() {
    LAST.with_borrow_mut(String::clear);
}

/// Why the last call failed, or "" after a success. It stays recorded.
pub fn get() -> String {
    LAST.with_borrow(String::clone)
}

/// Why the last call failed, or "" after a success, and the record emptied.
pub fn take() -> String {
    LAST.with_borrow_mut(std::mem::take)
}

/// The sentence for a call that could not `verb` the file `path` (as the program wrote it):
/// "could not open `save.bin`: the system cannot find the file specified".
pub fn sentence(verb: &str, path: &[u8], why: &str) -> String {
    format!("could not {verb} `{}`: {why}", String::from_utf8_lossy(path))
}

/// The operating system's words for `e`, as the end of a sentence: the first letter lower-cased,
/// without the error number Rust adds (" (os error 2)") or a closing period.
pub fn os_words(e: &io::Error) -> String {
    let text = e.to_string();
    let text = match text.rfind(" (os error ") {
        Some(at) if text.ends_with(')') => &text[..at],
        _ => &text[..],
    };
    let text = text.trim().trim_end_matches('.').trim_end();
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_lowercase().chain(chars).collect(),
        None => "the operating system gave no reason".to_string(),
    }
}

/// `set`s the sentence for an `io::Error` from trying to `verb` `path`.
pub fn set_io(verb: &str, path: &[u8], e: &io::Error) {
    set(sentence(verb, path, &os_words(e)));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_os_words_end_a_sentence() {
        let missing = io::Error::from_raw_os_error(2);
        let words = os_words(&missing);
        assert!(words.starts_with(|c: char| c.is_lowercase()), "{words}");
        assert!(!words.ends_with('.') && !words.contains("os error"), "{words}");
        assert_eq!(os_words(&io::Error::other("Not UTF-8.")), "not UTF-8");
        assert_eq!(os_words(&io::Error::other("")), "the operating system gave no reason");
    }

    #[test]
    fn a_sentence_names_the_path_as_written() {
        assert_eq!(sentence("open", b"save.bin", "it is missing"), "could not open `save.bin`: it is missing");
    }

    #[test]
    fn the_last_error_is_kept_until_the_next_call() {
        set("could not read `a`: no".to_string());
        assert_eq!(get(), "could not read `a`: no");
        assert_eq!(get(), "could not read `a`: no", "reading it leaves it");
        clear();
        assert_eq!(get(), "");
        set("x".to_string());
        assert_eq!(take(), "x");
        assert_eq!(get(), "", "taking it empties it");
    }
}
