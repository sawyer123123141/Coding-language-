//! The `args` and `env` modules' natives (`driver/std/args.kes`, `env.kes`): `kestrel_args_<name>`
//! and `kestrel_env_<name>`.
//!
//! The command line is what `start::set_args` was given (`kestrel run` sets the `.kes` path and the
//! words meant for the program), or else the process's own (a built executable, whose first word
//! is its path). Results are made in the current arena, and the checker never runs these calls in
//! a scratch arena, so the caller keeps them.

use std::ffi::OsString;

use super::abi::{str_bytes, string_list, write, write_none, write_some, KOpt};
use crate::arena::List;
use crate::string::KStr;

/// The arguments a replay hands the program in place of its own (`args.all()` under
/// `kestrel replay` gives the recorded ones). `None` when nothing is being replayed.
pub fn replay_args_override() -> Option<Vec<String>> {
    None
}

/// The program's path and its arguments: `set` when the host gave them, else split from `os`,
/// the process's command line, whose first word is the program. A word that isn't Unicode is
/// shown with U+FFFD in place of what isn't.
fn command_line(set: Option<&(String, Vec<String>)>, os: impl IntoIterator<Item = OsString>) -> (String, Vec<String>) {
    if let Some((program, args)) = set {
        return (program.clone(), args.clone());
    }
    let mut words = os.into_iter().map(|w| w.to_string_lossy().into_owned());
    let program = words.next().unwrap_or_default();
    (program, words.collect())
}

/// This process's program path and arguments.
fn this_command_line() -> (String, Vec<String>) {
    command_line(crate::start::args(), std::env::args_os())
}

/// The value of environment variable `name`, or `None` when it is unset, its value isn't UTF-8,
/// or the name is one no variable can have (empty, or holding `=` or a NUL).
fn variable(name: &[u8]) -> Option<String> {
    let name = std::str::from_utf8(name).ok()?;
    if name.is_empty() || name.contains(['=', '\0']) {
        return None;
    }
    std::env::var_os(name)?.into_string().ok()
}

/// `args.all() -> list[string]`: the words after the program's name.
///
/// # Safety
/// `out` must be valid for writing a `list[string]` (32 bytes).
#[no_mangle]
pub unsafe extern "C" fn kestrel_args_all(out: *mut List) {
    let args = replay_args_override().unwrap_or_else(|| this_command_line().1);
    write(out, string_list(&args));
}

/// `args.program() -> string`: the program's path as it was started.
///
/// # Safety
/// `out` must be valid for writing a string (16 bytes).
#[no_mangle]
pub unsafe extern "C" fn kestrel_args_program(out: *mut KStr) {
    KStr::make(this_command_line().0.as_bytes(), out);
}

/// `env.get(name) -> string?`.
///
/// # Safety
/// `out` must be valid for writing a `string?` (24 bytes); `name` must point to a live string.
#[no_mangle]
pub unsafe extern "C" fn kestrel_env_get(out: *mut KOpt<KStr>, name: *const KStr) {
    match variable(str_bytes(name)) {
        Some(value) => {
            let mut text = KStr { ptr: std::ptr::null(), len: 0 };
            KStr::make(value.as_bytes(), &mut text);
            write_some(out, text);
        }
        None => write_none(out),
    }
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;

    use super::{command_line, replay_args_override, variable};

    fn words(list: &[&str]) -> Vec<OsString> {
        list.iter().map(OsString::from).collect()
    }

    #[test]
    fn the_hosts_command_line_wins_over_the_processes() {
        let set = ("game.kes".to_string(), vec!["a".to_string(), "--b".to_string()]);
        assert_eq!(command_line(Some(&set), words(&["kestrel", "run", "game.kes"])), set);
    }

    #[test]
    fn an_executables_first_word_is_its_path() {
        assert_eq!(
            command_line(None, words(&["dir/game.exe", "a", "", "b c"])),
            ("dir/game.exe".to_string(), vec!["a".to_string(), String::new(), "b c".to_string()])
        );
        assert_eq!(command_line(None, words(&["game.exe"])), ("game.exe".to_string(), vec![]));
        assert_eq!(command_line(None, words(&[])), (String::new(), vec![]));
    }

    #[test]
    fn nothing_is_replayed_yet() {
        assert_eq!(replay_args_override(), None);
    }

    #[test]
    fn variables_are_read_and_impossible_names_are_none() {
        // `PATH` is set wherever tests run.
        assert!(variable(b"PATH").is_some());
        assert_eq!(variable(b"KESTREL_SURELY_UNSET_VARIABLE_42"), None);
        for name in [&b""[..], b"A=B", b"=", b"A\0B", b"\xFF"] {
            assert_eq!(variable(name), None, "{name:?}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_word_that_is_not_unicode_is_shown_with_replacements() {
        use std::os::unix::ffi::OsStringExt;
        let os = vec![OsString::from("p"), OsString::from_vec(vec![b'a', 0xFF])];
        assert_eq!(command_line(None, os).1, ["a\u{FFFD}"]);
    }
}
