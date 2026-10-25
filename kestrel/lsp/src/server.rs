//! The message loop. Requests are answered in order; edits only mark documents, which are
//! analysed when no message is waiting (so a burst of keystrokes is analysed once) or before
//! the next request (so it sees the latest text).

use std::collections::{HashMap, HashSet};
use std::panic::{catch_unwind, AssertUnwindSafe};

use crossbeam_channel::TryRecvError;
use lsp_server::{Connection, Message, Notification, Request, RequestId, Response};
use lsp_types::notification::{self as note, Notification as _};
use lsp_types::request::{self as req, Request as _};
use lsp_types::*;
use serde_json::Value;

use crate::config::Config;
use crate::diagnostics::{merged, Found};
use crate::docs::{key, Docs};
use crate::shift::LineShift;
use crate::{hints, nav, work};

pub struct Server {
    conn: Connection,
    pub(crate) docs: Docs,
    /// Per open document (its path's key): what checking its program found, by file.
    pub(crate) checked: HashMap<String, Found>,
    /// Per open document: what lowering found at the last open or save (contracts, assumptions
    /// and, when shown as diagnostics, the cost lens), by file.
    pub(crate) lowered: HashMap<String, Found>,
    /// The files that have diagnostics showing, and their URLs.
    shown: HashMap<String, Url>,
    pub(crate) config: Config,
    /// The editor can be asked to fetch inlay hints again.
    refresh_hints: bool,
    next_id: i32,
}

/// Runs the server on `conn` until the editor shuts it down.
pub fn run(conn: Connection) -> Result<(), String> {
    let caps = ServerCapabilities {
        text_document_sync: Some(TextDocumentSyncCapability::Options(TextDocumentSyncOptions {
            open_close: Some(true),
            change: Some(TextDocumentSyncKind::FULL),
            save: Some(TextDocumentSyncSaveOptions::SaveOptions(SaveOptions { include_text: Some(false) })),
            ..TextDocumentSyncOptions::default()
        })),
        hover_provider: Some(HoverProviderCapability::Simple(true)),
        definition_provider: Some(OneOf::Left(true)),
        inlay_hint_provider: Some(OneOf::Left(true)),
        ..ServerCapabilities::default()
    };
    let caps = serde_json::to_value(caps).map_err(|e| e.to_string())?;
    let params = conn.initialize(caps).map_err(|e| e.to_string())?;
    let params: InitializeParams = serde_json::from_value(params).map_err(|e| e.to_string())?;
    let mut config = Config::default();
    if let Some(options) = &params.initialization_options {
        config.update(options);
    }
    let refresh_hints = params
        .capabilities
        .workspace
        .and_then(|w| w.inlay_hint)
        .and_then(|h| h.refresh_support)
        .unwrap_or(false);
    let mut s = Server {
        conn,
        docs: Docs::default(),
        checked: HashMap::new(),
        lowered: HashMap::new(),
        shown: HashMap::new(),
        config,
        refresh_hints,
        next_id: 0,
    };
    s.main_loop()
}

impl Server {
    fn main_loop(&mut self) -> Result<(), String> {
        loop {
            let msg = match self.conn.receiver.try_recv() {
                Ok(m) => m,
                Err(TryRecvError::Empty) => {
                    self.flush();
                    match self.conn.receiver.recv() {
                        Ok(m) => m,
                        Err(_) => return Ok(()),
                    }
                }
                Err(TryRecvError::Disconnected) => return Ok(()),
            };
            match msg {
                Message::Request(r) => {
                    if self.conn.handle_shutdown(&r).map_err(|e| e.to_string())? {
                        return Ok(());
                    }
                    self.flush();
                    let response = self.request(r);
                    self.send(Message::Response(response));
                }
                // `exit` without `shutdown` first: stop anyway.
                Message::Notification(n) if n.method == note::Exit::METHOD => return Ok(()),
                Message::Notification(n) => self.notification(n),
                Message::Response(_) => {}
            }
        }
    }

    fn send(&self, m: Message) {
        // The editor is gone when this fails; the loop ends at the next receive.
        let _ = self.conn.sender.send(m);
    }

    fn request(&mut self, r: Request) -> Response {
        let id = r.id.clone();
        let result = match r.method.as_str() {
            req::HoverRequest::METHOD => parse(&r).map(|p: HoverParams| {
                let tp = p.text_document_position_params;
                let doc = self.docs.get(&tp.text_document.uri);
                let extra = doc.and_then(|d| hints::hover_text(d, tp.position.line));
                to_value(doc.and_then(|d| nav::hover(d, tp.position, extra)))
            }),
            req::GotoDefinition::METHOD => parse(&r).map(|p: GotoDefinitionParams| {
                let tp = p.text_document_position_params;
                let loc = self.docs.get(&tp.text_document.uri).and_then(|d| nav::definition(&self.docs, d, tp.position));
                to_value(loc.map(GotoDefinitionResponse::Scalar))
            }),
            req::InlayHintRequest::METHOD => parse(&r).map(|p: InlayHintParams| {
                let doc = self.docs.get(&p.text_document.uri);
                let list = match doc {
                    Some(d) if self.config.hints() => hints::inlay_hints(d, p.range),
                    _ => Vec::new(),
                };
                to_value(list)
            }),
            _ => return Response::new_err(id, lsp_server::ErrorCode::MethodNotFound as i32, format!("unknown request {}", r.method)),
        };
        match result {
            Ok(v) => Response::new_ok(id, v),
            Err(e) => Response::new_err(id, lsp_server::ErrorCode::InvalidParams as i32, e),
        }
    }

    fn notification(&mut self, n: Notification) {
        match n.method.as_str() {
            note::DidOpenTextDocument::METHOD => {
                if let Ok(p) = serde_json::from_value::<DidOpenTextDocumentParams>(n.params) {
                    let skip = p.text_document.uri.to_file_path().is_ok_and(|path| nav::is_builtin_copy(&path));
                    if !skip {
                        self.docs.open(p.text_document.uri, p.text_document.text);
                        // Its text may not be the file's on disk, which the others read until now.
                        self.docs.touch_all(false);
                    }
                }
            }
            note::DidChangeTextDocument::METHOD => {
                if let Ok(p) = serde_json::from_value::<DidChangeTextDocumentParams>(n.params) {
                    // Full sync: the last change is the whole text.
                    if let (Some(doc), Some(change)) = (self.docs.get_mut(&p.text_document.uri), p.content_changes.into_iter().last()) {
                        let text = kestrel_driver::without_bom(&change.text).to_string();
                        // What lowering found follows the edit until the next save.
                        let lines = LineShift::new(&doc.text, &text);
                        lines.lens(&mut doc.lens);
                        let k = key(&doc.path);
                        doc.text = text;
                        for found in self.lowered.values_mut() {
                            if let Some((_, diags)) = found.get_mut(&k) {
                                lines.diagnostics(diags);
                            }
                        }
                    }
                    self.docs.touch_all(false);
                }
            }
            note::DidSaveTextDocument::METHOD => self.docs.touch_all(true),
            note::DidCloseTextDocument::METHOD => {
                if let Ok(p) = serde_json::from_value::<DidCloseTextDocumentParams>(n.params) {
                    if let Some(doc) = self.docs.close(&p.text_document.uri) {
                        let k = key(&doc.path);
                        let mut files = self.files_of(&k);
                        files.insert(k.clone());
                        self.checked.remove(&k);
                        self.lowered.remove(&k);
                        self.publish(files);
                    }
                    // The others now read that file from the disk.
                    self.docs.touch_all(false);
                }
            }
            note::DidChangeConfiguration::METHOD => {
                if let Ok(p) = serde_json::from_value::<DidChangeConfigurationParams>(n.params) {
                    self.config.update(&p.settings);
                    self.docs.touch_all(true);
                }
            }
            _ => {}
        }
    }

    /// Every file `entry`'s findings have diagnostics in.
    fn files_of(&self, entry: &str) -> HashSet<String> {
        let mut out = HashSet::new();
        for found in [self.checked.get(entry), self.lowered.get(entry)].into_iter().flatten() {
            out.extend(found.keys().cloned());
        }
        out
    }

    /// Analyses every document marked for it, and shows what changed.
    fn flush(&mut self) {
        let dirty: Vec<Url> = self.docs.iter().filter(|d| d.dirty).map(|d| d.uri.clone()).collect();
        if dirty.is_empty() {
            return;
        }
        let mut files = HashSet::new();
        let mut lowered_any = false;
        for uri in dirty {
            let Some(doc) = self.docs.get(&uri) else { continue };
            let k = key(&doc.path);
            // Its own file always, so a clean one says so.
            files.insert(k.clone());
            files.extend(self.files_of(&k));
            let lower = doc.lower;
            let outcome = catch_unwind(AssertUnwindSafe(|| work::analyse(self, &uri, lower)));
            let outcome = outcome.unwrap_or_else(|panic| work::crashed(self, &uri, panic));
            lowered_any |= outcome;
            files.extend(self.files_of(&k));
        }
        self.publish(files);
        if lowered_any && self.refresh_hints {
            self.next_id += 1;
            let r = Request::new(RequestId::from(self.next_id), req::InlayHintRefreshRequest::METHOD.to_string(), Value::Null);
            self.send(Message::Request(r));
        }
    }

    /// Shows each file's diagnostics as they are now (none clears it).
    fn publish(&mut self, files: HashSet<String>) {
        for file in files {
            let all = self.checked.values().chain(self.lowered.values());
            let url = all.clone().find_map(|f| f.get(&file).map(|(u, _)| u.clone())).or_else(|| self.shown.get(&file).cloned()).or_else(|| self.docs.url_by_key(&file));
            let Some(url) = url else { continue };
            let diagnostics = merged(&file, all);
            if diagnostics.is_empty() {
                self.shown.remove(&file);
            } else {
                self.shown.insert(file.clone(), url.clone());
            }
            let params = PublishDiagnosticsParams { uri: url, diagnostics, version: None };
            let n = Notification::new(note::PublishDiagnostics::METHOD.to_string(), params);
            self.send(Message::Notification(n));
        }
    }
}

fn parse<P: serde::de::DeserializeOwned>(r: &Request) -> Result<P, String> {
    serde_json::from_value(r.params.clone()).map_err(|e| e.to_string())
}

fn to_value<T: serde::Serialize>(v: T) -> Value {
    serde_json::to_value(v).unwrap_or(Value::Null)
}
