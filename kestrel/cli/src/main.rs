mod driver;

use std::process::ExitCode;

use driver::DiagStyle;
use kestrel_codegen::{JitProgram, Options};

const USAGE: &str = "usage: kestrel <run|check> <file.kes> [--diagnostics=short]";

/// Turns any Rust panic in the compiler into exit 3, keeping 101 for Kestrel runtime panics.
fn install_panic_hook() {
    std::panic::set_hook(Box::new(|info| {
        eprintln!("kestrel: internal compiler error: {info}");
        std::process::exit(3);
    }));
}

fn main() -> ExitCode {
    install_panic_hook();
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

#[cfg(test)]
mod tests {
    use std::process::Command;

    const CHILD_ENV: &str = "KESTREL_PANIC_HOOK_CHILD";

    /// Child half of `compiler_panic_exits_3`; does nothing unless the parent set `CHILD_ENV`.
    #[test]
    fn panic_hook_child() {
        if std::env::var_os(CHILD_ENV).is_some() {
            super::install_panic_hook();
            panic!("boom");
        }
    }

    #[test]
    fn compiler_panic_exits_3() {
        let out = Command::new(std::env::current_exe().unwrap())
            .args(["tests::panic_hook_child", "--exact", "--nocapture"])
            .env(CHILD_ENV, "1")
            .output()
            .unwrap();
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert_eq!(out.status.code(), Some(3), "{stderr}");
        assert!(stderr.contains("kestrel: internal compiler error: "), "{stderr}");
        assert!(stderr.contains("boom") && stderr.contains("main.rs"), "{stderr}");
    }
}
