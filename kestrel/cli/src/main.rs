mod driver;
mod link;

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use driver::DiagStyle;
use kestrel_codegen::{emit_object, JitProgram, Options};

const USAGE: &str = "usage: kestrel <run|check|build> <file.kes> [-o <exe>] [--release] [--diagnostics=short]";

/// Turns any Rust panic in the compiler into exit 3, keeping 101 for Kestrel runtime panics.
fn install_panic_hook() {
    std::panic::set_hook(Box::new(|info| {
        eprintln!("kestrel: internal compiler error: {info}");
        std::process::exit(3);
    }));
}

struct Args {
    cmd: String,
    path: String,
    out: Option<String>,
    release: bool,
    style: DiagStyle,
}

fn parse_args(raw: Vec<String>) -> Option<Args> {
    let mut it = raw.into_iter();
    let (mut positional, mut out, mut release, mut style) = (Vec::new(), None, false, DiagStyle::Full);
    while let Some(a) = it.next() {
        match a.as_str() {
            "-o" => out = Some(it.next()?),
            "--release" => release = true,
            "--diagnostics=short" => style = DiagStyle::Short,
            s if s.starts_with('-') => return None,
            _ => positional.push(a),
        }
    }
    let [cmd, path]: [String; 2] = positional.try_into().ok()?;
    matches!(cmd.as_str(), "run" | "check" | "build").then_some(Args { cmd, path, out, release, style })
}

/// The compiler recurses over the syntax tree, so it runs on a stack far bigger than the
/// platform default.
const STACK_SIZE: usize = 256 * 1024 * 1024;

fn main() -> ExitCode {
    install_panic_hook();
    match std::thread::Builder::new().stack_size(STACK_SIZE).spawn(run) {
        Ok(worker) => worker.join().unwrap_or(ExitCode::from(3)),
        // Couldn't reserve the stack; the default one still handles ordinary programs.
        Err(_) => run(),
    }
}

/// Source text without a leading UTF-8 byte order mark, which editors on Windows add.
fn without_bom(src: &str) -> &str {
    src.strip_prefix('\u{feff}').unwrap_or(src)
}

/// True when both paths name the same file (compared canonically when both exist).
fn same_file(a: &Path, b: &Path) -> bool {
    match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        (Ok(a), Ok(b)) => a == b,
        _ => a == b,
    }
}

fn run() -> ExitCode {
    let Some(args) = parse_args(std::env::args().skip(1).collect()) else {
        eprintln!("{USAGE}");
        return ExitCode::from(2);
    };
    let src = match std::fs::read_to_string(&args.path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("kestrel: can't read {}: {e}", args.path);
            return ExitCode::from(2);
        }
    };
    let src = without_bom(&src);
    if args.cmd == "build" {
        if let Err(e) = check_output(&args) {
            eprintln!("kestrel: {e}");
            return ExitCode::from(2);
        }
    }
    let Some(module) = driver::compile(&args.path, src, args.style) else { return ExitCode::from(1) };
    let opts = Options { checked_arith: !args.release };
    let result = match args.cmd.as_str() {
        "check" => return ExitCode::SUCCESS,
        "run" => JitProgram::compile(&module, opts).map_err(internal_error).map(|p| p.run_main()),
        _ => build(&module, opts, &args).map(|()| 0),
    };
    match result {
        Ok(code) => ExitCode::from(code as u8),
        Err(e) => {
            eprintln!("kestrel: {e}");
            ExitCode::from(3)
        }
    }
}

fn internal_error(e: String) -> String {
    format!("internal compiler error: {e}")
}

/// Output path: `-o`, else `<source stem><EXE_SUFFIX>` in the current directory.
fn output_path(args: &Args) -> Result<PathBuf, String> {
    if let Some(o) = &args.out {
        return Ok(PathBuf::from(o));
    }
    let stem = Path::new(&args.path).file_stem().ok_or("source path has no file name")?;
    Ok(PathBuf::from(format!("{}{}", stem.to_string_lossy(), std::env::consts::EXE_SUFFIX)))
}

/// Refuses a `build` whose executable would overwrite the source file.
fn check_output(args: &Args) -> Result<(), String> {
    let exe = output_path(args)?;
    if same_file(&exe, Path::new(&args.path)) {
        return Err(format!("the output {} is the source file; pick another with -o", exe.display()));
    }
    Ok(())
}

fn build(module: &kestrel_kir::Module, opts: Options, args: &Args) -> Result<(), String> {
    let exe = output_path(args)?;
    let bytes = emit_object(module, opts).map_err(internal_error)?;
    let obj = std::env::temp_dir().join(format!("kestrel-{}.o", std::process::id()));
    std::fs::write(&obj, bytes).map_err(|e| format!("can't write {}: {e}", obj.display()))?;
    let linked = link::link(&obj, &exe);
    let _ = std::fs::remove_file(&obj);
    linked
}

#[cfg(test)]
mod tests {
    use std::process::Command;

    use super::{output_path, parse_args, without_bom};

    fn args(list: &[&str]) -> Option<super::Args> {
        parse_args(list.iter().map(|s| s.to_string()).collect())
    }

    #[test]
    fn default_output_is_the_stem_plus_exe_suffix() {
        let a = args(&["build", "dir/my.prog.kes"]).unwrap();
        let expected = format!("my.prog{}", std::env::consts::EXE_SUFFIX);
        assert_eq!(output_path(&a).unwrap().to_str().unwrap(), expected);
        let a = args(&["build", "x.kes", "-o", "out/y"]).unwrap();
        assert_eq!(output_path(&a).unwrap().to_str().unwrap(), "out/y");
    }

    #[test]
    fn bad_command_lines_are_rejected() {
        assert!(args(&["build", "x.kes", "--bogus"]).is_none());
        assert!(args(&["build", "x.kes", "-o"]).is_none());
        assert!(args(&["frob", "x.kes"]).is_none());
        assert!(args(&["build"]).is_none());
        assert!(args(&["build", "x.kes", "extra"]).is_none());
    }

    #[test]
    fn only_a_leading_byte_order_mark_is_stripped() {
        assert_eq!(without_bom("\u{feff}main() {}"), "main() {}");
        assert_eq!(without_bom("main() {}"), "main() {}");
        assert_eq!(without_bom("a\u{feff}"), "a\u{feff}");
    }

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
