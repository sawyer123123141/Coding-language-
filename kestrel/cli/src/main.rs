mod driver;

use std::process::ExitCode;

use driver::DiagStyle;
use kestrel_codegen::{JitProgram, Options};

const USAGE: &str = "usage: kestrel <run|check> <file.kes> [--diagnostics=short]";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let style = if args.iter().any(|a| a == "--diagnostics=short") { DiagStyle::Short } else { DiagStyle::Full };
    let positional: Vec<&str> = args.iter().filter(|a| !a.starts_with("--")).map(String::as_str).collect();
    let [cmd, path] = positional[..] else {
        eprintln!("{USAGE}");
        return ExitCode::from(2);
    };
    let src = match std::fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("kestrel: can't read {path}: {e}");
            return ExitCode::from(2);
        }
    };
    match cmd {
        "check" | "run" => {}
        _ => {
            eprintln!("{USAGE}");
            return ExitCode::from(2);
        }
    }
    let Some(module) = driver::compile(path, &src, style) else { return ExitCode::from(1) };
    if cmd == "check" {
        return ExitCode::SUCCESS;
    }
    match JitProgram::compile(&module, Options::default()) {
        Ok(program) => ExitCode::from(program.run_main() as u8),
        Err(e) => {
            eprintln!("kestrel: internal compiler error: {e}");
            ExitCode::from(3)
        }
    }
}
