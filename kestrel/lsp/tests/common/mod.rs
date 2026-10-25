//! Drives `kestrel-lsp` over standard input and output with real LSP messages.

use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{channel, Receiver};
use std::time::{Duration, Instant};

use lsp_types::Url;
use serde_json::{json, Value};

pub struct Client {
    child: Child,
    stdin: ChildStdin,
    incoming: Receiver<Value>,
    next_id: i64,
    /// Messages received while waiting for something else.
    pub seen: Vec<Value>,
}

/// A fresh directory holding `files` (name, text).
pub fn project(name: &str, files: &[(&str, &str)]) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("lsp_{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    for (file, text) in files {
        std::fs::write(dir.join(file), text).unwrap();
    }
    dir
}

pub fn url(path: &Path) -> Url {
    Url::from_file_path(path).unwrap()
}

impl Client {
    pub fn start(options: Value) -> Client {
        let mut child = Command::new(env!("CARGO_BIN_EXE_kestrel-lsp"))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .expect("start kestrel-lsp");
        let stdin = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();
        let (tx, incoming) = channel();
        std::thread::spawn(move || {
            let mut r = BufReader::new(stdout);
            loop {
                let mut len = 0;
                loop {
                    let mut line = String::new();
                    if r.read_line(&mut line).unwrap_or(0) == 0 {
                        return;
                    }
                    let line = line.trim_end();
                    if line.is_empty() {
                        break;
                    }
                    if let Some(n) = line.strip_prefix("Content-Length: ") {
                        len = n.parse().unwrap();
                    }
                }
                let mut body = vec![0; len];
                if r.read_exact(&mut body).is_err() {
                    return;
                }
                if tx.send(serde_json::from_slice(&body).unwrap()).is_err() {
                    return;
                }
            }
        });
        let mut c = Client { child, stdin, incoming, next_id: 0, seen: Vec::new() };
        let caps = json!({"workspace": {"inlayHint": {"refreshSupport": true}}});
        let r = c.request("initialize", json!({"processId": null, "rootUri": null, "capabilities": caps, "initializationOptions": options}));
        assert!(r["capabilities"]["hoverProvider"].as_bool().unwrap(), "{r}");
        c.notify("initialized", json!({}));
        c
    }

    fn write(&mut self, v: Value) {
        let body = v.to_string();
        write!(self.stdin, "Content-Length: {}\r\n\r\n{body}", body.len()).unwrap();
        self.stdin.flush().unwrap();
    }

    pub fn notify(&mut self, method: &str, params: Value) {
        self.write(json!({"jsonrpc": "2.0", "method": method, "params": params}));
    }

    /// Sends a request and waits for its response's result.
    pub fn request(&mut self, method: &str, params: Value) -> Value {
        self.next_id += 1;
        let id = self.next_id;
        self.write(json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}));
        let r = self.wait(|m| m["id"] == json!(id) && m.get("method").is_none());
        assert!(r.get("error").is_none(), "{r}");
        r["result"].clone()
    }

    /// The first message `want` accepts, kept or new (others are kept in `seen`); a server request is
    /// answered with an empty result.
    pub fn wait(&mut self, want: impl Fn(&Value) -> bool) -> Value {
        // Files' diagnostics come in any order: one may have arrived while waiting for another.
        if let Some(i) = self.seen.iter().position(&want) {
            return self.seen.remove(i);
        }
        let deadline = Instant::now() + Duration::from_secs(60);
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            let m = self.incoming.recv_timeout(left).unwrap_or_else(|_| panic!("timed out; seen {:#?}", self.seen));
            if want(&m) {
                return m;
            }
            if m.get("method").is_some() && m.get("id").is_some() {
                let id = m["id"].clone();
                self.write(json!({"jsonrpc": "2.0", "id": id, "result": null}));
            }
            self.seen.push(m);
        }
    }

    /// The first diagnostics published for `uri` that `want` accepts. Files are analysed again
    /// as other files change, so earlier ones may still be on their way.
    pub fn diagnostics(&mut self, uri: &Url, want: impl Fn(&[Value]) -> bool) -> Vec<Value> {
        let u = uri.as_str().to_string();
        let list = |m: &Value| m["params"]["diagnostics"].as_array().cloned().unwrap_or_default();
        let m = self.wait(|m| m["method"] == "textDocument/publishDiagnostics" && same_url(&m["params"]["uri"], &u) && want(&list(m)));
        list(&m)
    }

    pub fn open(&mut self, uri: &Url, text: &str) {
        let doc = json!({"uri": uri, "languageId": "kestrel", "version": 1, "text": text});
        self.notify("textDocument/didOpen", json!({"textDocument": doc}));
    }

    pub fn change(&mut self, uri: &Url, version: i32, text: &str) {
        let doc = json!({"uri": uri, "version": version});
        self.notify("textDocument/didChange", json!({"textDocument": doc, "contentChanges": [{"text": text}]}));
    }

    /// Shuts the server down and checks it exits cleanly.
    pub fn stop(mut self) {
        let r = self.request("shutdown", Value::Null);
        assert_eq!(r, Value::Null);
        self.notify("exit", Value::Null);
        let status = self.child.wait().unwrap();
        assert!(status.success(), "{status:?}");
    }
}

/// URLs compared as the files they name (an editor may spell a drive letter differently).
fn same_url(v: &Value, want: &str) -> bool {
    let (Some(a), Ok(b)) = (v.as_str().and_then(|s| Url::parse(s).ok()), Url::parse(want)) else { return false };
    match (a.to_file_path(), b.to_file_path()) {
        (Ok(a), Ok(b)) => a.to_string_lossy().to_lowercase() == b.to_string_lossy().to_lowercase(),
        _ => a == b,
    }
}
