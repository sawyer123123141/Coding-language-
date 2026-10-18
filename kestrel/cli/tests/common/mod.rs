//! Helpers shared by the CLI test binaries.
#![allow(dead_code)]

pub mod smoke;

use std::ffi::{OsStr, OsString};
use std::process::Command;

/// Takes every `KESTREL_*` variable out of the child's environment, so nothing left in the
/// shell that runs `cargo test` (`KESTREL_HEADLESS`, `KESTREL_HIDDEN`, `KESTREL_THREADS`,
/// `KESTREL_ARENA_REPORT`, ...) changes what a test sees. Then it sets `KESTREL_POISON=1`, so
/// every program a test runs fills freed arena memory with 0xDB and never reuses it (a use of
/// freed memory then fails the test instead of reading stale bytes). Set the other ones a test
/// wants after this: a later `env` on the same variable wins, so a program turns poisoning off
/// with `// env: KESTREL_POISON=0` (and says why in a comment).
pub fn hermetic(cmd: &mut Command) -> &mut Command {
    scrub(cmd, std::env::vars_os().map(|(key, _)| key)).env("KESTREL_POISON", "1")
}

/// `hermetic`, for these variable names (the ones the current process has).
pub fn scrub(cmd: &mut Command, names: impl Iterator<Item = OsString>) -> &mut Command {
    for name in names.filter(|n| is_kestrel_variable(n)) {
        cmd.env_remove(name);
    }
    cmd
}

/// Names are case-insensitive on Windows, so `kestrel_hidden` is the same variable.
fn is_kestrel_variable(name: &OsStr) -> bool {
    name.to_string_lossy().to_ascii_uppercase().starts_with("KESTREL_")
}
