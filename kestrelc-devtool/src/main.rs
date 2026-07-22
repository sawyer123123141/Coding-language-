// Local dev server for kestrelc: serves a browser UI that compiles and
// runs Kestrel source via the real kestrelc (in-process JIT where
// supported, native AOT subprocess fallback otherwise) -- see
// docs/superpowers/specs/2026-07-21-devtool-editor-design.md for why
// this exists instead of extending kestrel-editor.html's WASM path.

mod runner;

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

fn handle_request(mut request: tiny_http::Request) {
    let method = request.method().clone();
    let url = request.url().to_string();
    match (&method, url.as_str()) {
        (tiny_http::Method::Get, "/") => {
            let header = Header::from_bytes(&b"Content-Type"[..], &b"text/html; charset=utf-8"[..]).unwrap();
            let _ = request.respond(Response::from_string(UI_HTML).with_header(header));
        }
        (tiny_http::Method::Post, "/run") => {
            let mut body = String::new();
            if request.as_reader().read_to_string(&mut body).is_err() {
                let _ = request.respond(Response::from_string("bad request body").with_status_code(400));
                return;
            }
            // A local dev tool exists specifically for a user to type
            // arbitrary, exploratory, possibly-broken code into -- a
            // panic anywhere in kestrelc while compiling/running that
            // input must never take the whole server down (every Run
            // afterward would then fail to even connect, not just fail
            // to compile). runner::run_source already wraps the JIT
            // execution step itself in catch_unwind, but not
            // compilation -- this outer one is the real safety net,
            // covering every step (JIT compile, JIT run, AOT paths, and
            // anything added here later) in one place instead of
            // needing a matching catch_unwind added inside runner.rs
            // every time a new path is added.
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| runner::run_source(&body)))
                .unwrap_or_else(|payload| {
                    let msg = payload
                        .downcast_ref::<&str>()
                        .map(|s| s.to_string())
                        .or_else(|| payload.downcast_ref::<String>().cloned())
                        .unwrap_or_else(|| "kestrelc-devtool: internal error (unknown panic)".to_string());
                    runner::RunResult {
                        engine: "jit",
                        ok: false,
                        compile_ms: 0.0,
                        run_ms: 0.0,
                        output: String::new(),
                        error: Some(msg),
                    }
                });
            let header = Header::from_bytes(&b"Content-Type"[..], &b"application/json"[..]).unwrap();
            let _ = request.respond(Response::from_string(result.to_json()).with_header(header));
        }
        _ => {
            let _ = request.respond(Response::from_string("not found").with_status_code(404));
        }
    }
}
