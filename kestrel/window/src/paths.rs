//! Where relative paths in a program point.
//!
//! Under `kestrel run`, a program reads and writes its files next to its own
//! `.kes` file, wherever the command is run from. A built program has no source
//! file, so it uses the folder of the running executable. An absolute path is
//! always used as given. `window.screenshot` and `draw.load_texture` follow
//! this one rule.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// The folder `kestrel run` set, if it did.
static PROGRAM_DIR: OnceLock<PathBuf> = OnceLock::new();

/// Sets the folder relative paths are read from: the program's own folder under `kestrel run`.
/// The first call wins. A relative `dir` is made absolute against the current directory.
pub fn set_program_dir(dir: &Path) {
    set_in(&PROGRAM_DIR, dir);
}

/// `path` (UTF-8 bytes from Kestrel) as a file to read or write: an absolute path as given; a
/// relative one inside the program's folder, else the running executable's folder.
pub fn resolve(path: &[u8]) -> PathBuf {
    let exe_dir = std::env::current_exe().ok().and_then(|exe| exe.parent().map(Path::to_path_buf));
    resolve_in(path, PROGRAM_DIR.get().map(PathBuf::as_path), exe_dir.as_deref())
}

/// `set_program_dir` for the folder held in `cell`.
fn set_in(cell: &OnceLock<PathBuf>, dir: &Path) {
    let dir = if dir.as_os_str().is_empty() {
        // `kestrel run game.kes` has no folder in front of the file name: that is the current one.
        std::env::current_dir().ok()
    } else {
        std::path::absolute(dir).ok()
    };
    // If the current folder can't be found there is nothing better to do than leave the rule as it was.
    if let Some(dir) = dir {
        let _ = cell.set(dir);
    }
}

/// `resolve` with the folders given: the program's (if one was set) and the executable's (if known).
fn resolve_in(path: &[u8], program_dir: Option<&Path>, exe_dir: Option<&Path>) -> PathBuf {
    let path = PathBuf::from(String::from_utf8_lossy(path).into_owned());
    // A path with a root (`\data` on Windows) is not inside any folder either.
    if path.is_absolute() || path.has_root() {
        return path;
    }
    match program_dir.or(exe_dir) {
        Some(dir) => dir.join(path),
        None => path,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A path that is absolute on this platform.
    fn absolute(tail: &str) -> PathBuf {
        std::env::current_dir().unwrap().join(tail)
    }

    #[test]
    fn an_absolute_path_is_unchanged() {
        let path = absolute("pictures/a.png");
        let program = absolute("program");
        let exe = absolute("exe");
        let text = path.to_str().unwrap().as_bytes();
        assert_eq!(resolve_in(text, Some(&program), Some(&exe)), path);
        assert_eq!(resolve_in(text, None, Some(&exe)), path);
        assert_eq!(resolve_in(text, None, None), path);
    }

    #[test]
    fn a_relative_path_is_joined_to_the_set_folder() {
        let program = absolute("program");
        let exe = absolute("exe");
        assert_eq!(resolve_in(b"a.png", Some(&program), Some(&exe)), program.join("a.png"));
        assert_eq!(resolve_in(b"img/a.png", Some(&program), Some(&exe)), program.join("img/a.png"));
    }

    #[test]
    fn with_no_folder_set_the_executables_folder_is_used() {
        let exe = absolute("exe");
        assert_eq!(resolve_in(b"a.png", None, Some(&exe)), exe.join("a.png"));
    }

    #[test]
    fn with_no_folder_known_a_relative_path_is_left_alone() {
        assert_eq!(resolve_in(b"a.png", None, None), PathBuf::from("a.png"));
    }

    #[test]
    fn the_running_executables_folder_is_the_default() {
        // No test sets the program folder, so this is the test executable's folder.
        let exe_dir = std::env::current_exe().unwrap().parent().unwrap().to_path_buf();
        assert_eq!(resolve(b"a.png"), exe_dir.join("a.png"));
    }

    #[cfg(windows)]
    #[test]
    fn a_rooted_path_without_a_drive_is_not_moved_into_the_folder() {
        let program = absolute("program");
        assert_eq!(resolve_in(br"\data.png", Some(&program), None), PathBuf::from(r"\data.png"));
    }

    #[test]
    fn the_first_folder_set_wins_and_a_relative_one_is_made_absolute() {
        let cell = OnceLock::new();
        set_in(&cell, Path::new("some/where"));
        assert_eq!(cell.get(), Some(&absolute("some/where")));
        set_in(&cell, &absolute("elsewhere"));
        assert_eq!(cell.get(), Some(&absolute("some/where")), "the first call wins");
    }

    #[test]
    fn an_empty_folder_means_the_current_one() {
        // `kestrel run game.kes` has an empty parent folder.
        let cell = OnceLock::new();
        set_in(&cell, Path::new(""));
        assert_eq!(cell.get(), Some(&std::env::current_dir().unwrap()));
    }
}
