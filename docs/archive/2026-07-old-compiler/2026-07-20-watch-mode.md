# `kestrelc watch` Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a `kestrelc watch <file.kes>` subcommand: on every save, recompile and rerun the file, so testing a `.kes` program no longer requires manually re-running `kestrelc file.kes && ./file` after each edit.

**Architecture:** A new `kestrelc/src/watch.rs` module owns the watch loop. It uses the `notify` crate to watch one file, debounces rapid-fire change events from a single save, then shells out to the *current kestrelc executable itself* (`std::env::current_exe()`) to compile — reusing the exact same tested compile path `kestrelc file.kes` already uses, rather than duplicating compiler-pipeline logic in-process. On a successful compile it runs the resulting binary and streams its output. `main.rs` gains a small subcommand dispatch (`kestrelc watch <path>` vs. the existing `kestrelc [--wasm] <path>`).

**Tech Stack:** Rust, `notify` crate (new dependency, native-feature-gated), `std::process::Command`, `std::sync::mpsc`.

## Global Constraints

- Native backend only — no `--wasm` support in watch mode (design doc's explicit scope decision).
- One file at a time — no directory/multi-file watching.
- The watcher must never crash on a compile error — it prints the error and keeps waiting for the next save.
- `notify` must be gated behind the existing `native` Cargo feature (same as the Cranelift dependencies), so it doesn't leak into `kestrelc-web`'s `wasm32-unknown-unknown` lib build, which uses `default-features = false`.
- Debounce window: 100ms (design doc's stated value) — a burst of file-write events from one save must coalesce into a single recompile.
- Reuse the current `kestrelc` executable for compilation (via `std::env::current_exe()` + `Command`) rather than duplicating `main.rs`'s inline compile pipeline — there is no existing reusable native-compile function in `lib.rs` to call in-process (confirmed: `lib.rs` only exposes `compile_to_wasm_bytes`, no native equivalent), so self-invoking the binary is the correct way to avoid duplicating that logic, not a workaround.

---

### Task 1: `watch.rs` — debounce logic and the watch loop

**Files:**
- Create: `kestrelc/src/watch.rs`
- Modify: `kestrelc/Cargo.toml` (add `notify` dependency, native-feature-gated)
- Modify: `kestrelc/src/lib.rs` (declare `pub mod watch;`, native-feature-gated)

**Interfaces:**
- Produces: `pub fn run(path: &str) -> std::process::ExitCode` — the subcommand's entry point, called by `main.rs` in Task 2.
- Produces (for testability): `pub(crate) fn drain_debounced(rx: &std::sync::mpsc::Receiver<()>, timeout: std::time::Duration) -> bool` — blocks until either the receiver is disconnected (returns `false`) or one-or-more `()` events have arrived and then gone quiet for `timeout` (returns `true`). This is the debounce logic extracted into something a unit test can drive without a real filesystem watcher.

- [ ] **Step 1: Add the `notify` dependency to `Cargo.toml`**

Find in `kestrelc/Cargo.toml`:

```toml
native = [
    "dep:cranelift-codegen",
    "dep:cranelift-frontend",
    "dep:cranelift-module",
    "dep:cranelift-object",
    "dep:cranelift-native",
    "dep:target-lexicon",
]

[dependencies]
cranelift-codegen = { version = "0.116", optional = true }
cranelift-frontend = { version = "0.116", optional = true }
cranelift-module = { version = "0.116", optional = true }
cranelift-object = { version = "0.116", optional = true }
cranelift-native = { version = "0.116", optional = true }
target-lexicon = { version = "0.12", optional = true }
wasm-encoder = "0.253.0"
```

Replace with:

```toml
native = [
    "dep:cranelift-codegen",
    "dep:cranelift-frontend",
    "dep:cranelift-module",
    "dep:cranelift-object",
    "dep:cranelift-native",
    "dep:target-lexicon",
    "dep:notify",
]

[dependencies]
cranelift-codegen = { version = "0.116", optional = true }
cranelift-frontend = { version = "0.116", optional = true }
cranelift-module = { version = "0.116", optional = true }
cranelift-object = { version = "0.116", optional = true }
cranelift-native = { version = "0.116", optional = true }
target-lexicon = { version = "0.12", optional = true }
wasm-encoder = "0.253.0"
# `kestrelc watch` only -- file-system watching for the native-only watch
# subcommand. Gated behind "native" like the Cranelift deps above, for
# the same reason (kestrelc-web's wasm32 lib build has no use for it and
# notify's OS-level watching wouldn't compile there anyway).
notify = { version = "6", optional = true }
```

- [ ] **Step 2: Write the failing debounce test**

Create `kestrelc/src/watch.rs` with just enough to compile the test (the real `run` function comes in Step 4):

```rust
use std::sync::mpsc::Receiver;
use std::time::Duration;

/// Blocks until either the channel disconnects (returns `false`) or at
/// least one event has arrived and then no further event arrives for
/// `timeout` (returns `true`) -- coalesces a burst of rapid-fire events
/// from one logical file save into a single "go" signal.
pub(crate) fn drain_debounced(rx: &Receiver<()>, timeout: Duration) -> bool {
    if rx.recv().is_err() {
        return false;
    }
    loop {
        match rx.recv_timeout(timeout) {
            Ok(()) => continue,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => return true,
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => return false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc::channel;
    use std::thread;

    #[test]
    fn a_burst_of_events_coalesces_into_one_debounced_signal() {
        let (tx, rx) = channel::<()>();
        thread::spawn(move || {
            for _ in 0..5 {
                tx.send(()).unwrap();
                thread::sleep(Duration::from_millis(5));
            }
            // sender drops here; no more events after the burst
        });
        // First call should return true once the burst goes quiet.
        assert!(drain_debounced(&rx, Duration::from_millis(50)));
        // The channel is now empty and disconnected (sender dropped) --
        // the next call should observe disconnection, not hang.
        assert!(!drain_debounced(&rx, Duration::from_millis(50)));
    }

    #[test]
    fn a_disconnected_channel_with_no_events_returns_false_immediately() {
        let (tx, rx) = channel::<()>();
        drop(tx);
        assert!(!drain_debounced(&rx, Duration::from_millis(50)));
    }
}
```

- [ ] **Step 3: Run the test to verify it passes (this is a rare case where the minimal implementation is written in the same step as the test, since the function is small and self-contained enough that a separate "verify it fails first" step would only be testing a typo)**

```bash
cd kestrelc
cargo test --features native drain_debounced 2>&1 | tail -20
```

Expected: both tests PASS.

- [ ] **Step 4: Add the watch loop and `run` entry point to `watch.rs`**

Append to `kestrelc/src/watch.rs` (above the existing `#[cfg(test)]` block -- Rust doesn't care about order, but keep tests at the bottom of the file by convention):

```rust
use notify::{Event, EventKind, RecursiveMode, Watcher};
use std::path::Path;
use std::process::{Command, ExitCode};
use std::sync::mpsc::channel;

/// `kestrelc watch <file.kes>` -- on every save, recompile and rerun.
///
/// Shells out to the current `kestrelc` executable rather than calling
/// the compiler pipeline in-process: this is the exact same code path
/// `kestrelc <file.kes>` already uses and already has tests for, so
/// watch mode can't drift from normal compile behavior.
pub fn run(path: &str) -> ExitCode {
    let src_path = Path::new(path);
    if !src_path.exists() {
        eprintln!("kestrelc: can't read '{path}': No such file or directory");
        return ExitCode::FAILURE;
    }
    let stem = match src_path.file_stem() {
        Some(s) => s.to_string_lossy().into_owned(),
        None => {
            eprintln!("kestrelc: '{path}' has no file stem");
            return ExitCode::FAILURE;
        }
    };
    let exe = match std::env::current_exe() {
        Ok(e) => e,
        Err(e) => {
            eprintln!("kestrelc: can't find my own executable path: {e}");
            return ExitCode::FAILURE;
        }
    };

    let (tx, rx) = channel::<()>();
    let mut watcher = match notify::recommended_watcher(move |res: notify::Result<Event>| {
        if let Ok(event) = res {
            if matches!(event.kind, EventKind::Modify(_) | EventKind::Create(_)) {
                let _ = tx.send(());
            }
        }
    }) {
        Ok(w) => w,
        Err(e) => {
            eprintln!("kestrelc: failed to start file watcher: {e}");
            return ExitCode::FAILURE;
        }
    };
    if let Err(e) = watcher.watch(src_path, RecursiveMode::NonRecursive) {
        eprintln!("kestrelc: failed to watch '{path}': {e}");
        return ExitCode::FAILURE;
    }

    println!("kestrelc: watching {path} (Ctrl+C to stop)");
    compile_and_run(&exe, path, &stem);

    const DEBOUNCE: std::time::Duration = std::time::Duration::from_millis(100);
    while drain_debounced(&rx, DEBOUNCE) {
        compile_and_run(&exe, path, &stem);
    }
    ExitCode::SUCCESS
}

fn compile_and_run(exe: &Path, path: &str, stem: &str) {
    print!("\x1B[2J\x1B[1;1H"); // clear screen, move cursor to top-left
    println!("kestrelc watch: {path}");

    let compile_status = Command::new(exe).arg(path).status();
    match compile_status {
        Ok(status) if status.success() => {}
        Ok(_) => return, // compiler already printed its own error
        Err(e) => {
            eprintln!("kestrelc: failed to invoke self ('{}'): {e}", exe.display());
            return;
        }
    }

    // Matches link_and_report's own output naming (kestrelc/src/main.rs)
    // exactly: `-o <stem>`, no extension appended, same as this
    // project's own integration tests already invoke the compiled
    // binary by.
    let bin_path = format!("./{stem}");
    println!("--- running {bin_path} ---");
    match Command::new(&bin_path).status() {
        Ok(status) => println!("--- exited with {status} ---"),
        Err(e) => eprintln!("kestrelc: failed to run '{bin_path}': {e}"),
    }
}
```

- [ ] **Step 5: Declare the module in `lib.rs`**

Find in `kestrelc/src/lib.rs`:

```rust
#[cfg(feature = "native")]
pub mod profile;
```

Replace with:

```rust
#[cfg(feature = "native")]
pub mod profile;
#[cfg(feature = "native")]
pub mod watch;
```

- [ ] **Step 6: Build and run the full test suite**

```bash
cd kestrelc
export PATH="/c/Users/sawye/AppData/Local/Microsoft/WinGet/Packages/BrechtSanders.WinLibs.POSIX.UCRT_Microsoft.Winget.Source_8wekyb3d8bbwe/mingw64/bin:$PATH"
cargo build --features native 2>&1 | tail -40
cargo test 2>&1 | tail -20
```

Expected: clean build, everything green (this task adds no new integration-level behavior yet -- `watch.rs` isn't wired into `main.rs` until Task 2 -- so no existing test should be affected).

- [ ] **Step 7: Commit**

```bash
git add kestrelc/Cargo.toml kestrelc/src/lib.rs kestrelc/src/watch.rs
git commit -m "Add kestrelc watch's core loop (not yet wired into the CLI)

notify-based file watcher with a debounced compile-and-run loop.
Shells out to the current kestrelc executable to compile, reusing the
exact same tested compile path kestrelc <file.kes> already uses.
Not reachable from the CLI yet -- that's the next commit."
```

---

### Task 2: Wire `kestrelc watch <file.kes>` into the CLI

**Files:**
- Modify: `kestrelc/src/main.rs`
- Test: `kestrelc/tests/integration.rs`

**Interfaces:**
- Consumes: `kestrelc::watch::run(path: &str) -> ExitCode` from Task 1.
- Produces: nothing new for later tasks -- this is the last task.

- [ ] **Step 1: Write the failing integration test**

Add to `kestrelc/tests/integration.rs`, anywhere near the other CLI-invocation tests (a reasonable spot: near the top-level "usage" or basic-compile tests -- check the existing file for where a bare `kestrelc <bad-args>` usage-error test lives, if one exists, and put this near it):

```rust
#[test]
fn watch_rejects_a_nonexistent_file_immediately_without_hanging() {
    // This test only exercises the fast-fail path (Task 1 Step 4's
    // `!src_path.exists()` check) -- it must never enter the actual
    // watch loop, which would hang a test suite forever. Watch mode's
    // interactive recompile-on-save behavior isn't practical to cover
    // in an automated integration test (it never exits on its own by
    // design), so this is the one thing about `watch` this suite can
    // safely assert on.
    let scratch = scratch_dir("watch_missing_file");
    let missing_path = scratch.join("does_not_exist.kes");

    let out = Command::new(kestrelc_bin())
        .arg("watch")
        .arg(&missing_path)
        .current_dir(&scratch)
        .output()
        .expect("failed to run kestrelc");

    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("No such file or directory"), "got: {stderr}");
}
```

- [ ] **Step 2: Run it to verify it fails**

```bash
cd kestrelc
export PATH="/c/Users/sawye/AppData/Local/Microsoft/WinGet/Packages/BrechtSanders.WinLibs.POSIX.UCRT_Microsoft.Winget.Source_8wekyb3d8bbwe/mingw64/bin:$PATH"
cargo test watch_rejects_a_nonexistent_file 2>&1 | tail -30
```

Expected: FAIL -- `kestrelc watch <path>` currently falls through to the normal `[--wasm] <path>` argument parser (since `watch` isn't special-cased yet), which will try to read `watch` as a flag or `<path>` literally as a filename, producing a different error path than the one Task 1's `watch::run` produces, or possibly a "usage" error instead. Either way this confirms the dispatch doesn't exist yet.

- [ ] **Step 3: Add the subcommand dispatch to `main.rs`**

Find in `kestrelc/src/main.rs`:

```rust
fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    let (wasm, path) = match args.as_slice() {
        [_, flag, path] if flag == "--wasm" => (true, path.clone()),
        [_, path] => (false, path.clone()),
        _ => {
            eprintln!("usage: kestrelc [--wasm] <file.kes>");
            return ExitCode::FAILURE;
        }
    };
```

Replace with:

```rust
fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    if let [_, cmd, path] = args.as_slice() {
        if cmd == "watch" {
            return kestrelc::watch::run(path);
        }
    }
    let (wasm, path) = match args.as_slice() {
        [_, flag, path] if flag == "--wasm" => (true, path.clone()),
        [_, path] => (false, path.clone()),
        _ => {
            eprintln!("usage: kestrelc [--wasm] <file.kes>\n       kestrelc watch <file.kes>");
            return ExitCode::FAILURE;
        }
    };
```

- [ ] **Step 4: Run the new integration test**

```bash
cd kestrelc
cargo test watch_rejects_a_nonexistent_file 2>&1 | tail -20
```

Expected: PASS.

- [ ] **Step 5: Run the full test suite**

```bash
cargo test 2>&1 | tail -20
```

Expected: everything green.

- [ ] **Step 6: Manual smoke test (not automatable -- do this yourself, report the outcome in your commit/report, don't skip it)**

```bash
cd kestrelc
export PATH="/c/Users/sawye/AppData/Local/Microsoft/WinGet/Packages/BrechtSanders.WinLibs.POSIX.UCRT_Microsoft.Winget.Source_8wekyb3d8bbwe/mingw64/bin:$PATH"
cargo build --features native
mkdir -p /tmp/watch-smoke && cd /tmp/watch-smoke
echo 'fn main() { print("hello"); }' > smoke.kes
"$OLDPWD/target/debug/kestrelc" watch smoke.kes &
WATCH_PID=$!
sleep 2   # let it compile+run once
echo 'fn main() { print("hello again"); }' > smoke.kes
sleep 2   # let it detect the change and recompile+rerun
kill $WATCH_PID
```

Confirm in the captured output: it ran once printing "hello", detected the edit, recompiled, and ran again printing "hello again" -- without the process crashing or hanging in between. Then confirm the error-recovery path separately: edit `smoke.kes` to contain a syntax error while a `kestrelc watch` is running, confirm it prints the error and doesn't exit; fix the error, confirm it recovers and runs again.

- [ ] **Step 7: Commit**

```bash
git add kestrelc/src/main.rs kestrelc/tests/integration.rs
git commit -m "Wire kestrelc watch <file.kes> into the CLI

kestrelc watch now recompiles and reruns on every save, using the
watch loop added in the previous commit. Manually verified the
recompile-on-save loop and the compile-error-recovery path (not
practical to cover with an automated integration test, since watch
mode never exits on its own by design)."
```

---

## After this plan

Ship the way this session has been shipping everything: work on a feature branch off `main`, merge back with `--no-ff` (or fast-forward, matching this repo's established pattern) once tests are green.
