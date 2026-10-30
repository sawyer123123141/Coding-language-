//! `kestrel run --record <file.replay>` and `kestrel replay <file.kes> <file.replay>` (the I/O
//! round's spec, decision 7). The recorder and the player are the runtime's
//! (`kestrel_runtime::replay`); this starts them around `main` and reports what they found.

use std::path::Path;

use kestrel_runtime::replay::{self, Ending, Recording, Report, WindowMode};
use kestrel_runtime::replay_file::ReplayFile;

use crate::Args;

/// The compiler's version, which a recording names.
const VERSION: &str = env!("CARGO_PKG_VERSION");

/// The build a recording names: `checked`, or `release` for `--release`.
fn build_name(args: &Args) -> &'static str {
    if args.release {
        "release"
    } else {
        "checked"
    }
}

/// Reads the replay file `file` for `replay`, and notes on stderr when it was recorded from
/// another program file, Kestrel or build (the replay still runs: the bits may still agree).
pub fn read(file: &str, args: &Args) -> Result<ReplayFile, String> {
    let read = ReplayFile::read(Path::new(file))?;
    if let Some(note) = read.mismatch_note(file, &args.path, VERSION, build_name(args)) {
        eprintln!("kestrel: {note}");
    }
    Ok(read)
}

/// Starts replaying `replaying`, or recording when `--record` was given. Nothing otherwise.
pub fn start(args: &Args, replaying: Option<ReplayFile>) {
    if let Some(file) = replaying {
        replay::start_replay(file);
    } else if let Some(path) = &args.record {
        // Settings the window can't read make `window.open` fail with their own message; a
        // program that opens no window doesn't care, and its recording says `real`.
        let window = kestrel_window::window_mode().unwrap_or(WindowMode::Real);
        replay::start_recording(Recording {
            path: path.into(),
            program: args.path.clone(),
            version: VERSION.to_string(),
            build: build_name(args).to_string(),
            window,
            args: args.program_args.clone(),
        });
    }
}

/// Ends recording or replaying once `main` has returned with exit code `code`: writes the
/// recording, or prints the replay's report and gives its exit code (0 matches, 1 differs).
pub fn finish(code: i32) -> Result<i32, String> {
    match replay::finish(Ending::Returned)? {
        Report::Off | Report::Recorded => Ok(code),
        report => {
            println!("{}", report.text().unwrap_or_default());
            Ok(report.exit_code())
        }
    }
}
