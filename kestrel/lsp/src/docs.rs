//! The open documents: their unsaved text, which the compiler reads in place of the file on disk,
//! and what was last found in each.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use kestrel_driver::Analysis;
use lsp_types::Url;

use crate::lens::LensLine;

/// One open document.
pub struct Doc {
    pub uri: Url,
    pub path: PathBuf,
    pub text: String,
    /// The latest analysis of the program with this document as its entry file.
    pub analysis: Option<Analysis>,
    /// The latest analysis that had tables, and the text it was made from: hover and definition
    /// fall back to it while the document doesn't parse.
    pub good: Option<(String, Analysis)>,
    /// What the cost lens found at the last save (empty when it is off or hasn't run).
    pub lens: Vec<LensLine>,
    /// Needs analysing again.
    pub dirty: bool,
    /// Needs its contracts and its cost lens again (it, or a file it uses, was opened or saved).
    pub lower: bool,
}

#[derive(Default)]
pub struct Docs {
    docs: HashMap<String, Doc>,
}

/// A path as a key: on Windows, paths differ only in case and in which slash they use.
pub fn key(path: &Path) -> String {
    let s = path.to_string_lossy();
    if cfg!(windows) {
        s.replace('/', "\\").to_lowercase()
    } else {
        s.into_owned()
    }
}

impl Docs {
    pub fn open(&mut self, uri: Url, text: String) {
        let Ok(path) = uri.to_file_path() else { return };
        let text = kestrel_driver::without_bom(&text).to_string();
        let doc = Doc { uri, path: path.clone(), text, analysis: None, good: None, lens: Vec::new(), dirty: true, lower: true };
        self.docs.insert(key(&path), doc);
    }

    pub fn close(&mut self, uri: &Url) -> Option<Doc> {
        let path = uri.to_file_path().ok()?;
        self.docs.remove(&key(&path))
    }

    pub fn get(&self, uri: &Url) -> Option<&Doc> {
        self.docs.get(&key(&uri.to_file_path().ok()?))
    }

    pub fn get_mut(&mut self, uri: &Url) -> Option<&mut Doc> {
        self.docs.get_mut(&key(&uri.to_file_path().ok()?))
    }

    /// The URL of the open document whose path has key `k` (see `key`).
    pub fn url_by_key(&self, k: &str) -> Option<Url> {
        self.docs.get(k).map(|d| d.uri.clone())
    }

    pub fn by_path(&self, path: &Path) -> Option<&Doc> {
        self.docs.get(&key(path))
    }

    pub fn iter(&self) -> impl Iterator<Item = &Doc> {
        self.docs.values()
    }

    /// The text of the module file at `path`: the open document's, else the file on disk.
    pub fn read(&self, path: &Path) -> Option<String> {
        if let Some(d) = self.by_path(path) {
            return Some(d.text.clone());
        }
        let text = std::fs::read_to_string(path).ok()?;
        Some(kestrel_driver::without_bom(&text).to_string())
    }

    /// Marks every document for analysing again (a change to one may change what another
    /// that uses it finds), and, with `lower`, for its contracts and cost lens.
    pub fn touch_all(&mut self, lower: bool) {
        for d in self.docs.values_mut() {
            d.dirty = true;
            d.lower |= lower;
        }
    }
}

/// The URL of a source file of an analysis: the open document's own, when it is open (its
/// spelling is the editor's), else made from the path.
pub fn url_of(docs: &Docs, path: &Path) -> Option<Url> {
    match docs.by_path(path) {
        Some(d) => Some(d.uri.clone()),
        None => Url::from_file_path(path).ok(),
    }
}
