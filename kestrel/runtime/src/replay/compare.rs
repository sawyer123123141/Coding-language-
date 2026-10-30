//! The output log: what the program printed, split into lines as it is printed. A recording keeps
//! the lines; a replay compares each line with the recorded one as soon as it is complete, and
//! the last, unfinished line and the way the program ended when it ends.

use crate::replay_file::Entry;

/// How the program ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Ending {
    /// `main` returned (after the window closed, or the headless frames ran out, or with no window).
    Returned,
    /// A runtime failure stopped it, with this message.
    Failure(String),
}

/// What `finish` found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Report {
    /// Nothing was being recorded or replayed.
    Off,
    /// The replay file was written.
    Recorded,
    /// The replay ran as recorded: `replay matches (N frames, M lines)`.
    Matches { frames: u64, lines: usize },
    /// The replay differs, as this message says.
    Differs(String),
}

impl Report {
    /// The line (or lines) `kestrel replay` prints, if any.
    pub fn text(&self) -> Option<String> {
        match self {
            Report::Off | Report::Recorded => None,
            Report::Matches { frames, lines } => Some(format!("replay matches ({frames} frames, {lines} lines)")),
            Report::Differs(msg) => Some(msg.clone()),
        }
    }

    /// The exit code of `kestrel replay`: 1 when the replay differs, else 0.
    pub fn exit_code(&self) -> i32 {
        i32::from(matches!(self, Report::Differs(_)))
    }
}

/// Splits printed bytes into lines: `pending` holds what came after the last line break, and
/// `each` is given every line completed by `bytes`, without its line break.
pub fn split<E>(pending: &mut Vec<u8>, bytes: &[u8], mut each: impl FnMut(&[u8]) -> Result<(), E>) -> Result<(), E> {
    let mut rest = bytes;
    while let Some(i) = rest.iter().position(|&b| b == b'\n') {
        if pending.is_empty() {
            each(&rest[..i])?;
        } else {
            pending.extend_from_slice(&rest[..i]);
            let line = std::mem::take(pending);
            each(&line)?;
        }
        rest = &rest[i + 1..];
    }
    pending.extend_from_slice(rest);
    Ok(())
}

/// What this run printed or did, to compare with the next entry of the output log.
#[derive(Clone, Copy)]
enum Got<'a> {
    Line(&'a [u8]),
    Partial(&'a [u8]),
    Failure(&'a str),
    End,
}

/// How a report shows a line, an unfinished line, a failure or the end of the output.
fn shown(got: Got) -> String {
    match got {
        Got::Line(text) => String::from_utf8_lossy(text).into_owned(),
        Got::Partial(text) => format!("{} <no line break>", String::from_utf8_lossy(text)),
        Got::Failure(msg) => format!("<failure: {msg}>"),
        Got::End => "<end of output>".to_string(),
    }
}

fn of_entry(entry: Option<&Entry>) -> Got<'_> {
    match entry {
        Some(Entry::Line(text)) => Got::Line(text),
        Some(Entry::Partial(text)) => Got::Partial(text),
        Some(Entry::Failure(msg)) => Got::Failure(msg),
        None => Got::End,
    }
}

fn same(a: Got, b: Got) -> bool {
    match (a, b) {
        (Got::Line(a), Got::Line(b)) | (Got::Partial(a), Got::Partial(b)) => a == b,
        (Got::Failure(a), Got::Failure(b)) => a == b,
        (Got::End, Got::End) => true,
        _ => false,
    }
}

/// A replay's place in the recorded output log.
#[derive(Default)]
pub struct Check {
    /// The next entry to compare.
    next: usize,
    /// What this run printed after its last line break.
    pending: Vec<u8>,
}

impl Check {
    /// Compares `got` with the next recorded entry, at frame `frame`.
    fn compare(&mut self, output: &[Entry], got: Got, frame: u64) -> Result<(), String> {
        let recorded = of_entry(output.get(self.next));
        if !same(recorded, got) {
            return Err(format!(
                "replay differs at frame {frame}, line {}:\n  recorded: {}\n  this run: {}",
                self.next + 1,
                shown(recorded),
                shown(got)
            ));
        }
        self.next += 1;
        Ok(())
    }

    /// Compares each line `bytes` completes.
    pub fn print(&mut self, output: &[Entry], bytes: &[u8], frame: u64) -> Result<(), String> {
        let mut pending = std::mem::take(&mut self.pending);
        let result = split(&mut pending, bytes, |line| self.compare(output, Got::Line(line), frame));
        self.pending = pending;
        result
    }

    /// Compares the end of the output: the unfinished line, then how the program ended. Gives the
    /// number of lines compared.
    pub fn end(&mut self, output: &[Entry], ending: &Ending, frame: u64) -> Result<usize, String> {
        let pending = std::mem::take(&mut self.pending);
        if !pending.is_empty() {
            self.compare(output, Got::Partial(&pending), frame)?;
        }
        let lines = self.next;
        if let Ending::Failure(msg) = ending {
            self.compare(output, Got::Failure(msg), frame)?;
        }
        self.compare(output, Got::End, frame)?;
        Ok(lines)
    }
}

/// The output log a recording keeps.
#[derive(Default)]
pub struct Keep {
    pub entries: Vec<Entry>,
    pending: Vec<u8>,
}

impl Keep {
    pub fn print(&mut self, bytes: &[u8]) {
        let entries = &mut self.entries;
        let _ = split::<()>(&mut self.pending, bytes, |line| {
            entries.push(Entry::Line(line.to_vec()));
            Ok(())
        });
    }

    /// Ends the log: the unfinished line, then the failure if the program failed.
    pub fn end(&mut self, ending: &Ending) {
        if !self.pending.is_empty() {
            self.entries.push(Entry::Partial(std::mem::take(&mut self.pending)));
        }
        if let Ending::Failure(msg) = ending {
            self.entries.push(Entry::Failure(msg.clone()));
        }
    }
}
