//! The server's settings, sent by the editor as `kestrel.*` (`initializationOptions` at start,
//! then `workspace/didChangeConfiguration`).

use serde_json::Value;

/// How the cost lens shows what it found.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Show {
    /// An inlay hint at the end of the line.
    Hints,
    /// An Information diagnostic on the line.
    Diagnostics,
    Both,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Config {
    /// Whether the cost lens runs (on open and save).
    pub lens: bool,
    /// The checks of a checked build (`kestrel run`), or of a `--release` one.
    pub checked: bool,
    pub show: Show,
}

impl Default for Config {
    fn default() -> Self {
        Config { lens: true, checked: true, show: Show::Hints }
    }
}

impl Config {
    /// Reads `{"costLens": {"enabled", "build", "show"}}`, either bare or under `kestrel`; what
    /// is missing or not understood keeps its current value.
    pub fn update(&mut self, v: &Value) {
        let v = v.get("kestrel").unwrap_or(v);
        let Some(lens) = v.get("costLens") else { return };
        if let Some(on) = lens.get("enabled").and_then(Value::as_bool) {
            self.lens = on;
        }
        match lens.get("build").and_then(Value::as_str) {
            Some("checked") => self.checked = true,
            Some("release") => self.checked = false,
            _ => {}
        }
        match lens.get("show").and_then(Value::as_str) {
            Some("hints") => self.show = Show::Hints,
            Some("diagnostics") => self.show = Show::Diagnostics,
            Some("both") => self.show = Show::Both,
            _ => {}
        }
    }

    pub fn hints(&self) -> bool {
        self.lens && matches!(self.show, Show::Hints | Show::Both)
    }

    pub fn diagnostics(&self) -> bool {
        self.lens && matches!(self.show, Show::Diagnostics | Show::Both)
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{Config, Show};

    #[test]
    fn settings_are_read_bare_or_under_kestrel() {
        let mut c = Config::default();
        c.update(&json!({"kestrel": {"costLens": {"enabled": false, "build": "release", "show": "both"}}}));
        assert_eq!(c, Config { lens: false, checked: false, show: Show::Both });
        c.update(&json!({"costLens": {"enabled": true, "show": "nonsense"}}));
        assert_eq!(c, Config { lens: true, checked: false, show: Show::Both });
        assert!(c.hints() && c.diagnostics());
    }
}
