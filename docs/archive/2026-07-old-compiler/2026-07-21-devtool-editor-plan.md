# kestrelc-devtool Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A local server + browser UI that compiles and runs Kestrel source via the real `kestrelc` (in-process JIT where supported, native AOT subprocess fallback otherwise), reporting real, separately-measured compile time and run time.

**Architecture:** New independent crate `kestrelc-devtool/` (sibling to `kestrelc/`/`kestrelc-web/`, no workspace — matches existing repo layout), depending on `kestrelc` (native feature) + `tiny_http`. Serves one embedded HTML page and one `POST /run` JSON endpoint.

**Tech Stack:** Rust, `tiny_http` (new dependency), `kestrelc` (path dependency), plain HTML/CSS/JS (no framework, no build step).

## Global Constraints

- No Stop button / interrupt capability — confirmed out of scope (see design doc).
- No WASM path — this tool exists specifically because WASM gives misleading numbers for this purpose.
- No server-side state between requests — every `/run` compiles fresh.
- `kestrelc watch`'s existing behavior/output must be completely unaffected — any change to shared code (`jit_codegen.rs`) must be purely additive (new functions, not modified existing ones).
- Full design doc: `docs/superpowers/specs/2026-07-21-devtool-editor-design.md`.

---

### Task 1: Scaffold the crate — server boots, opens browser, serves a placeholder page

**Files:**
- Create: `kestrelc-devtool/Cargo.toml`
- Create: `kestrelc-devtool/src/main.rs`
- Create: `kestrelc-devtool/ui.html` (placeholder content for now, real UI in Task 4)

**Interfaces:**
- Produces: a runnable binary that starts an HTTP server on `127.0.0.1:7420`, serves `ui.html`'s contents at `GET /`, and opens the default browser to `http://127.0.0.1:7420/` on launch.

- [ ] **Step 1: Create the crate**

`kestrelc-devtool/Cargo.toml`:
```toml
[package]
name = "kestrelc-devtool"
version = "0.1.0"
edition = "2021"

[dependencies]
kestrelc = { path = "../kestrelc" }
tiny_http = "0.12"
```

`kestrelc-devtool/ui.html` (placeholder — replaced in Task 4):
```html
<!DOCTYPE html>
<html><body><h1>kestrelc-devtool placeholder</h1></body></html>
```

- [ ] **Step 2: Write the server skeleton**

`kestrelc-devtool/src/main.rs`:
```rust
// Local dev server for kestrelc: serves a browser UI that compiles and
// runs Kestrel source via the real kestrelc (in-process JIT where
// supported, native AOT subprocess fallback otherwise) -- see
// docs/superpowers/specs/2026-07-21-devtool-editor-design.md for why
// this exists instead of extending kestrel-editor.html's WASM path.

use std::process::Command;
use tiny_http::{Header, Response, Server};

const PORT: u16 = 7420;
const UI_HTML: &str = include_str!("../ui.html");

fn main() {
    let addr = format!("127.0.0.1:{PORT}");
    let server = Server::http(&addr).unwrap_or_else(|e| {
        eprintln!("kestrelc-devtool: couldn't bind {addr}: {e}");
        std::process::exit(1);
    });

    let url = format!("http://{addr}/");
    println!("kestrelc-devtool: listening on {url}");
    open_browser(&url);

    for request in server.incoming_requests() {
        handle_request(request);
    }
}

// Windows-only (matches this project's other platform-specific spots,
// e.g. kestrelc's own MSVC/mingw notes) -- `cmd /c start` is the
// standard way to open the default browser without a new dependency
// (an `open`-crate equivalent), same "no dependency for something this
// small" posture as elsewhere in this project.
fn open_browser(url: &str) {
    if let Err(e) = Command::new("cmd").args(["/c", "start", "", url]).status() {
        eprintln!("kestrelc-devtool: couldn't auto-open a browser ({e}) -- open {url} manually");
    }
}

fn handle_request(request: tiny_http::Request) {
    let method = request.method().clone();
    let url = request.url().to_string();
    match (&method, url.as_str()) {
        (tiny_http::Method::Get, "/") => {
            let header = Header::from_bytes(&b"Content-Type"[..], &b"text/html; charset=utf-8"[..]).unwrap();
            let _ = request.respond(Response::from_string(UI_HTML).with_header(header));
        }
        _ => {
            let _ = request.respond(Response::from_string("not found").with_status_code(404));
        }
    }
}
```

- [ ] **Step 3: Verify it runs**

Run: `cd kestrelc-devtool && cargo run`
Expected: prints `kestrelc-devtool: listening on http://127.0.0.1:7420/`, the default browser opens showing the placeholder page.

- [ ] **Step 4: Commit**

```bash
git add kestrelc-devtool/
git commit -m "Scaffold kestrelc-devtool: server boots, opens browser, serves a placeholder page"
```

---

### Task 2: Add opt-in stdout capture to `jit_codegen.rs` (purely additive)

**Files:**
- Modify: `kestrelc/src/jit_codegen.rs`

**Interfaces:**
- Consumes: nothing new from other tasks.
- Produces: `JitCodegen::new_capturing() -> Result<Self, KestrelcError>` (alternate constructor; existing `JitCodegen::new()` is completely unchanged — `kestrelc watch` keeps using it, unaffected), and two free functions `jit_codegen::begin_capture()` / `jit_codegen::take_captured_output() -> String`, used by Task 3.

**Why this is needed:** `finish_and_run`'s JIT-executed code writes through the real libc `printf` (an `extern "C"` FFI import — see the existing module doc comment) directly to the process's real stdout. The devtool needs that output back as a string to send in the JSON response, not printed to the devtool server's own console. Rather than OS-level stdout-handle redirection (real risk: this project already hit a hard MSVC/mingw CRT mismatch once tonight — see the module's own doc comment on the reverted `build.rs` approach — so hand-rolling more CRT-internal tricks is exactly the kind of fragile territory worth avoiding), this registers a *different* `printf`-symbol implementation, opt-in, only for `JitCodegen::new_capturing()`. `gen_print`/`call_printf` (this file) only ever pass one of a small, fully-known set of format strings to this symbol (`"%s\n"`, `"%s "`, `"%lld\n"`, `"%lld "`, or a bare `"\n"` for `print()` with no args) — never arbitrary user-controlled format strings — so the capture function only needs to handle those specific shapes, not general `printf`.

- [ ] **Step 1: Write the failing test**

Add to `jit_codegen.rs`'s existing `#[cfg(test)] mod tests`:
```rust
#[test]
fn captured_output_matches_what_print_would_normally_write_to_real_stdout() {
    jit_codegen::begin_capture();
    let program = parse(lex(
        "fn main() {\n\
         \x20   print(\"hello\", 42, \"world\");\n\
         \x20   print(7);\n\
         \x20   print();\n\
         \x20   return 1;\n\
         }\n",
    ).unwrap()).unwrap();
    check_jit_supported(&program).unwrap();
    let mut cg = JitCodegen::new_capturing().unwrap();
    cg.compile_program(&program).unwrap();
    let result = cg.finish_and_run().unwrap();
    assert_eq!(result, 1);
    let output = take_captured_output();
    assert_eq!(output, "hello 42 world\n7\n\n");
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd kestrelc && cargo test --lib jit_codegen:: captured_output -- --test-threads=1`
Expected: FAIL — `new_capturing` not found (doesn't exist yet).

- [ ] **Step 3: Implement the capture buffer and alternate printf symbol**

Near the top of `jit_codegen.rs`, alongside the existing `kestrelc_jit_abort`/`kestrelc_jit_enter_frame` host functions:
```rust
use std::cell::RefCell;
use std::ffi::CStr;

thread_local! {
    static CAPTURE_BUFFER: RefCell<Option<String>> = const { RefCell::new(None) };
}

/// Starts (or restarts) output capture for the next `JitCodegen::new_capturing`
/// run on this thread. Call before `compile_program`/`finish_and_run` --
/// `kestrelc_jit_capture_printf` below appends to this buffer instead of
/// writing to real stdout, for the lifetime of one compile-and-run cycle.
pub fn begin_capture() {
    CAPTURE_BUFFER.with(|b| *b.borrow_mut() = Some(String::new()));
}

/// Takes (and clears) whatever was captured since the last `begin_capture`.
/// Returns an empty string if capture was never started -- callers that
/// forgot `begin_capture` get silence, not a panic, matching this file's
/// existing "never a hard failure for something recoverable" posture.
pub fn take_captured_output() -> String {
    CAPTURE_BUFFER.with(|b| b.borrow_mut().take().unwrap_or_default())
}

/// Alternate `printf` symbol registered only by `JitCodegen::new_capturing`
/// (see `new_impl` below) -- `gen_print`/`call_printf` in this same file
/// only ever pass one of a small, fully-known set of format strings here
/// (never arbitrary/user-controlled ones), so this only needs to handle
/// those exact shapes, not general printf. Any other format string is
/// unreachable given this file's own call sites; the wildcard arm exists
/// so a future change to those call sites fails loud (empty output) rather
/// than reading uninitialized/mismatched varargs.
extern "C" fn kestrelc_jit_capture_printf(fmt: *const u8, arg: i64) -> i32 {
    let fmt_bytes = unsafe { CStr::from_ptr(fmt as *const std::ffi::c_char) }.to_bytes();
    let text = match fmt_bytes {
        b"%s\n" => format!("{}\n", read_c_str(arg)),
        b"%s " => format!("{} ", read_c_str(arg)),
        b"%lld\n" => format!("{arg}\n"),
        b"%lld " => format!("{arg} "),
        b"\n" => "\n".to_string(),
        _ => String::new(),
    };
    CAPTURE_BUFFER.with(|b| {
        if let Some(buf) = b.borrow_mut().as_mut() {
            buf.push_str(&text);
        }
    });
    0
}

fn read_c_str(ptr: i64) -> String {
    unsafe { CStr::from_ptr(ptr as *const std::ffi::c_char) }.to_string_lossy().into_owned()
}
```

- [ ] **Step 4: Add `new_capturing`, refactor `new`'s body into a shared `new_impl`**

In `impl JitCodegen`, change:
```rust
pub fn new() -> Result<Self, KestrelcError> {
```
to:
```rust
pub fn new() -> Result<Self, KestrelcError> {
    Self::new_impl(printf as *const u8)
}

/// Same as `new`, except JIT-executed `print()` output is captured into
/// a thread-local buffer (see `begin_capture`/`take_captured_output`
/// above) instead of going to the real process stdout. Used only by
/// kestrelc-devtool; `kestrelc watch`'s existing behavior (via `new`) is
/// completely unaffected.
pub fn new_capturing() -> Result<Self, KestrelcError> {
    Self::new_impl(kestrelc_jit_capture_printf as *const u8)
}

fn new_impl(printf_symbol: *const u8) -> Result<Self, KestrelcError> {
```
...and inside that function body, change the existing line
```rust
        jit_builder.symbol("printf", printf as *const u8);
```
to:
```rust
        jit_builder.symbol("printf", printf_symbol);
```

- [ ] **Step 5: Run test to verify it passes**

Run: `cd kestrelc && cargo test --lib jit_codegen:: -- --test-threads=1`
Expected: PASS, including the new test and every existing `jit_codegen` test unchanged (confirms `new()`'s default real-`printf` behavior is untouched).

- [ ] **Step 6: Run the full suite**

Run: `cd kestrelc && cargo test -- --test-threads=1`
Expected: PASS, same count as before plus 1.

- [ ] **Step 7: Commit**

```bash
git add kestrelc/src/jit_codegen.rs
git commit -m "Add opt-in JIT stdout capture (JitCodegen::new_capturing), for kestrelc-devtool"
```

---

### Task 3: `/run` endpoint — real compile-and-run orchestration with split timing

**Files:**
- Create: `kestrelc-devtool/src/runner.rs`
- Modify: `kestrelc-devtool/src/main.rs` (wire in the new route)
- Modify: `kestrelc-devtool/Cargo.toml` (no new deps needed — JSON is hand-encoded per the design doc)

**Interfaces:**
- Consumes: `kestrelc::{lexer, parser, resolve, purity, typecheck, jit_codegen}` (public library functions, same ones `watch.rs` already calls), `kestrelc::interner::well_known::main`.
- Produces: `pub fn run_source(src: &str) -> RunResult` and `pub struct RunResult { engine: &'static str, ok: bool, compile_ms: f64, run_ms: f64, output: String, error: Option<String> }`, plus `impl RunResult { pub fn to_json(&self) -> String }`.

- [ ] **Step 1: Write the failing tests**

`kestrelc-devtool/src/runner.rs` (new file), test module at the bottom:
```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_jit_eligible_program_runs_via_the_jit_engine_with_real_output() {
        let result = run_source("fn main() { print(\"hi\", 42); }");
        assert_eq!(result.engine, "jit");
        assert!(result.ok);
        assert_eq!(result.output, "hi 42\n");
        assert!(result.compile_ms >= 0.0);
        assert!(result.run_ms >= 0.0);
        assert!(result.error.is_none());
    }

    #[test]
    fn a_jit_ineligible_program_falls_back_to_the_aot_engine() {
        let result = run_source("fn main() { let arr = [1, 2, 3]; print(arr[0]); }");
        assert_eq!(result.engine, "aot");
        assert!(result.ok);
        assert_eq!(result.output.trim(), "1");
    }

    #[test]
    fn a_program_with_a_compile_error_reports_ok_false_with_the_real_diagnostic() {
        let result = run_source("fn main() { let x = ; }");
        assert!(!result.ok);
        assert!(result.error.as_ref().unwrap().contains("Unexpected"));
    }

    #[test]
    fn to_json_produces_valid_well_shaped_json() {
        let result = RunResult {
            engine: "jit",
            ok: true,
            compile_ms: 1.5,
            run_ms: 0.25,
            output: "hi \"there\"\n".to_string(),
            error: None,
        };
        let json = result.to_json();
        assert!(json.contains("\"engine\":\"jit\""));
        assert!(json.contains("\"ok\":true"));
        assert!(json.contains("\"compile_ms\":1.5"));
        // Embedded quote must be escaped, not break the JSON.
        assert!(json.contains("hi \\\"there\\\"\\n"));
        assert!(json.contains("\"error\":null"));
    }
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cd kestrelc-devtool && cargo test`
Expected: FAIL to compile (`run_source`/`RunResult` don't exist yet).

- [ ] **Step 3: Implement `RunResult` and JSON encoding**

Top of `kestrelc-devtool/src/runner.rs`:
```rust
// Compile-and-run orchestration for kestrelc-devtool's /run endpoint --
// mirrors kestrelc/src/watch.rs's try_jit-then-AOT-fallback structure,
// calling the same public kestrelc library functions watch.rs already
// does (watch.rs's own try_jit/report_error are private to that module,
// so this is a fresh call to the same underlying pipeline, not a reuse
// of watch.rs's glue -- see the design doc). Real, separately-timed
// compile_ms/run_ms is the one thing watch.rs doesn't already expose
// (it only reports one combined "finished in Xms").

use kestrelc::error::KestrelcError;
use kestrelc::{jit_codegen, lexer, parser, purity, resolve, typecheck};
use std::process::Command;
use std::time::Instant;

pub struct RunResult {
    pub engine: &'static str,
    pub ok: bool,
    pub compile_ms: f64,
    pub run_ms: f64,
    pub output: String,
    pub error: Option<String>,
}

impl RunResult {
    pub fn to_json(&self) -> String {
        format!(
            "{{\"engine\":\"{}\",\"ok\":{},\"compile_ms\":{},\"run_ms\":{},\"output\":\"{}\",\"error\":{}}}",
            self.engine,
            self.ok,
            self.compile_ms,
            self.run_ms,
            json_escape(&self.output),
            match &self.error {
                Some(e) => format!("\"{}\"", json_escape(e)),
                None => "null".to_string(),
            },
        )
    }
}

fn json_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out
}
```

- [ ] **Step 4: Implement `run_source`'s front-end pipeline (shared by both engines)**

Still in `runner.rs`:
```rust
pub fn run_source(src: &str) -> RunResult {
    let tokens = match lexer::lex(src) {
        Ok(t) => t,
        Err(e) => return compile_error(&e, src),
    };
    let program = match parser::parse(tokens) {
        Ok(p) => p,
        Err(e) => return compile_error(&e, src),
    };

    let fns = resolve::build_fn_table(&program);
    let structs = resolve::build_struct_table(&program);
    let resolve_errors = resolve::resolve(&program, &fns, &structs);
    if let Some(e) = resolve_errors.first() {
        return compile_error(e, src);
    }
    let purity_errors = purity::check_purity(&program, &fns);
    if let Some(e) = purity_errors.first() {
        return compile_error(e, src);
    }
    let pmap_errors = purity::check_parallel_map(&program, &fns);
    if let Some(e) = pmap_errors.first() {
        return compile_error(e, src);
    }
    let type_errors = typecheck::check_types(&program, &fns);
    if let Some(e) = type_errors.first() {
        return compile_error(e, src);
    }
    if !program.fns.iter().any(|f| f.name == kestrelc::interner::well_known::main()) {
        return RunResult {
            engine: "jit",
            ok: false,
            compile_ms: 0.0,
            run_ms: 0.0,
            output: String::new(),
            error: Some("kestrelc: No 'main' function found".to_string()),
        };
    }

    match jit_codegen::check_jit_supported(&program) {
        Ok(()) => run_via_jit(&program),
        Err(_) => run_via_aot(src),
    }
}

fn compile_error(e: &KestrelcError, src: &str) -> RunResult {
    let message = if e.span.line == 0 {
        format!("kestrelc: {}", e.message)
    } else {
        format!(
            "kestrelc: {}",
            kestrelc::format_diagnostic(src, "<devtool>", e.span.line, e.span.col, e.span.len.max(1), &e.message)
        )
    };
    RunResult { engine: "jit", ok: false, compile_ms: 0.0, run_ms: 0.0, output: String::new(), error: Some(message) }
}
```

**Design note on `compile_error`'s hardcoded `engine: "jit"`:** a front-end error (lex/parse/resolve/purity/type) happens before either engine is chosen -- both engines would hit the identical error, so the field is cosmetic here and never shown differently to the user (the UI only reads `error`, not `engine`, when `ok` is false). Not worth a third `engine` value for this.

- [ ] **Step 5: Implement `run_via_jit`**

```rust
fn run_via_jit(program: &kestrelc::ast::Program) -> RunResult {
    let compile_start = Instant::now();
    let mut cg = match jit_codegen::JitCodegen::new_capturing() {
        Ok(c) => c,
        Err(e) => return RunResult { engine: "jit", ok: false, compile_ms: 0.0, run_ms: 0.0, output: String::new(), error: Some(e.message) },
    };
    if let Err(e) = cg.compile_program(program) {
        return RunResult { engine: "jit", ok: false, compile_ms: 0.0, run_ms: 0.0, output: String::new(), error: Some(e.message) };
    }
    let compile_ms = compile_start.elapsed().as_secs_f64() * 1000.0;

    jit_codegen::begin_capture();
    let run_start = Instant::now();
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| cg.finish_and_run()));
    let run_ms = run_start.elapsed().as_secs_f64() * 1000.0;
    let output = jit_codegen::take_captured_output();

    match result {
        Ok(Ok(_)) => RunResult { engine: "jit", ok: true, compile_ms, run_ms, output, error: None },
        Ok(Err(e)) => RunResult { engine: "jit", ok: false, compile_ms, run_ms, output, error: Some(e.message) },
        Err(panic_payload) => {
            let msg = panic_payload
                .downcast_ref::<&str>()
                .map(|s| s.to_string())
                .or_else(|| panic_payload.downcast_ref::<String>().cloned())
                .unwrap_or_else(|| "unknown panic in JIT backend".to_string());
            RunResult { engine: "jit", ok: false, compile_ms, run_ms, output, error: Some(msg) }
        }
    }
}
```

(Same `catch_unwind` pattern `watch.rs`'s `try_jit` already uses, for the same reason: a bug in the JIT backend itself shouldn't take the whole devtool server down.)

- [ ] **Step 6: Implement `run_via_aot`**

```rust
fn run_via_aot(src: &str) -> RunResult {
    let dir = std::env::temp_dir().join(format!("kestrelc_devtool_{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let src_path = dir.join("prog.kes");
    if let Err(e) = std::fs::write(&src_path, src) {
        return RunResult { engine: "aot", ok: false, compile_ms: 0.0, run_ms: 0.0, output: String::new(), error: Some(format!("kestrelc-devtool: couldn't write temp file: {e}")) };
    }
    let kestrelc_exe = match std::env::current_exe().ok().and_then(|p| p.parent().map(|d| d.join("kestrelc.exe"))) {
        Some(p) if p.exists() => p,
        _ => return RunResult { engine: "aot", ok: false, compile_ms: 0.0, run_ms: 0.0, output: String::new(), error: Some("kestrelc-devtool: couldn't find kestrelc.exe next to this binary -- build kestrelc first (cargo build --release -p kestrelc) and copy/symlink it alongside kestrelc-devtool.exe".to_string()) },
    };

    let compile_start = Instant::now();
    let compile_output = match Command::new(&kestrelc_exe).arg(&src_path).current_dir(&dir).output() {
        Ok(o) => o,
        Err(e) => return RunResult { engine: "aot", ok: false, compile_ms: 0.0, run_ms: 0.0, output: String::new(), error: Some(format!("kestrelc-devtool: failed to invoke kestrelc: {e}")) },
    };
    let compile_ms = compile_start.elapsed().as_secs_f64() * 1000.0;
    if !compile_output.status.success() {
        return RunResult {
            engine: "aot", ok: false, compile_ms, run_ms: 0.0, output: String::new(),
            error: Some(String::from_utf8_lossy(&compile_output.stderr).into_owned()),
        };
    }

    let bin_path = dir.join("prog");
    let run_start = Instant::now();
    let run_output = Command::new(&bin_path).output();
    let run_ms = run_start.elapsed().as_secs_f64() * 1000.0;
    match run_output {
        Ok(o) => RunResult { engine: "aot", ok: true, compile_ms, run_ms, output: String::from_utf8_lossy(&o.stdout).into_owned(), error: None },
        Err(e) => RunResult { engine: "aot", ok: false, compile_ms, run_ms, output: String::new(), error: Some(format!("kestrelc-devtool: failed to run compiled binary: {e}")) },
    }
}
```

- [ ] **Step 7: Run tests to verify they pass**

Run: `cd kestrelc-devtool && cargo test`
Expected: PASS, all 4.

**Note:** `a_jit_ineligible_program_falls_back_to_the_aot_engine` requires `kestrelc.exe` built and discoverable next to the test binary's location, which won't hold in a plain `cargo test` run (test binaries live in `target/debug/deps/`, not next to a `kestrelc.exe`). Adjust the test to build kestrelc first and either set an env var the runner checks, or skip/ignore this specific test with a clear `#[ignore]` + comment pointing at manual verification (Task 5) instead of a fragile path-discovery test. Resolve this pragmatically during implementation -- don't force a brittle path-guessing test to pass artificially.

- [ ] **Step 8: Wire the route into `main.rs`**

In `kestrelc-devtool/src/main.rs`, add `mod runner;` at the top, and extend `handle_request`:
```rust
        (tiny_http::Method::Post, "/run") => {
            let mut body = String::new();
            let mut request = request;
            use std::io::Read;
            if request.as_reader().read_to_string(&mut body).is_err() {
                let _ = request.respond(Response::from_string("bad request body").with_status_code(400));
                return;
            }
            let result = runner::run_source(&body);
            let header = Header::from_bytes(&b"Content-Type"[..], &b"application/json"[..]).unwrap();
            let _ = request.respond(Response::from_string(result.to_json()).with_header(header));
        }
```

- [ ] **Step 9: Commit**

```bash
git add kestrelc-devtool/
git commit -m "Implement /run: real compile-and-run via JIT/AOT with split timing and JSON output"
```

---

### Task 4: The actual UI

**Files:**
- Modify: `kestrelc-devtool/ui.html` (replace placeholder)

**Interfaces:**
- Consumes: `POST /run` (Task 3), returns the JSON shape from `RunResult::to_json`.

- [ ] **Step 1: Write the real page**

Replace `kestrelc-devtool/ui.html` with a self-contained page: a `<textarea>` for source (pre-filled with a small sample program), a "Run" button, an output pane showing `output`/`error`, and a status line showing `engine`, `compile_ms`, `run_ms`. Visually modeled on `kestrel-editor.html`'s existing dark theme (reuse its `:root` CSS custom properties for colors/fonts — `--bg`, `--surface`, `--text`, `--rust`, `--wing`, etc. — copy those values directly rather than re-deriving a new palette) but written fresh, no WASM/`kestrel.js` script tags or loading logic at all.

Core JS (illustrative shape, not exhaustive — implementer fills in matching styling):
```html
<script>
async function run() {
  const src = document.getElementById('code').value;
  const statusEl = document.getElementById('status');
  const outEl = document.getElementById('output');
  statusEl.textContent = 'running...';
  const res = await fetch('/run', { method: 'POST', body: src });
  const data = await res.json();
  statusEl.textContent = `${data.engine} — compile ${data.compile_ms.toFixed(2)}ms, run ${data.run_ms.toFixed(2)}ms`;
  outEl.textContent = data.ok ? data.output : data.error;
  outEl.className = data.ok ? 'ok' : 'err';
}
document.getElementById('run-btn').addEventListener('click', run);
</script>
```

- [ ] **Step 2: Manual verification**

Run: `cd kestrelc-devtool && cargo build --release -p kestrelc && cp ../kestrelc/target/release/kestrelc.exe target/debug/kestrelc.exe && cargo run` (build `kestrelc.exe` and place it next to the devtool binary for the AOT fallback path — the exact copy/symlink step should be double-checked/adjusted for whatever `current_exe()` actually resolves to during a real `cargo run`, since debug vs release target dirs differ; get this right during implementation rather than assuming).

Then in the opened browser:
1. Paste the same "hi 42" print program used to verify JIT watch mode earlier this session. Click Run. Confirm `engine: jit`, both times are small real numbers, output is `hi 42`.
2. Paste a program with an array literal. Click Run. Confirm `engine: aot`, correct output, real (larger, subprocess-inclusive) timing.
3. Paste `fn main() { let x = ; }`. Click Run. Confirm the real formatted diagnostic appears, not a generic error.

- [ ] **Step 3: Commit**

```bash
git add kestrelc-devtool/ui.html
git commit -m "Build the real devtool UI: editor, run button, output pane, split timing display"
```

---

### Task 5: Whole-branch review and finish

- [ ] Dispatch a final code review (or self-review if working solo) covering: the `jit_codegen.rs` capture addition (confirm zero behavior change to `JitCodegen::new()`'s existing callers), the AOT fallback's temp-file/subprocess handling (cleanup? left temp files are a minor real issue worth at least noting), JSON escaping correctness, and the manual verification steps from Task 4.
- [ ] Use `superpowers:finishing-a-development-branch` to decide merge/PR/cleanup.
