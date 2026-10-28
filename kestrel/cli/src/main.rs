mod link;

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use kestrel_driver::{without_bom, DiagStyle};
use kestrel_codegen::{emit_object, has_sse41, JitProgram, Options, NO_SSE41, TARGET_CPUS};
use link::Lib;

const USAGE: &str = "usage: kestrel <run|check|build> <file.kes> [-o <exe>] [--release] [--report] [--diagnostics=short]";

/// What `--report` on anything but `check` says.
const REPORT_NEEDS_CHECK: &str = "`--report` goes with `check`: kestrel check --report file.kes";

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
    report: bool,
    style: DiagStyle,
}

fn parse_args(raw: Vec<String>) -> Option<Args> {
    let mut it = raw.into_iter();
    let (mut positional, mut out, mut release, mut report, mut style) = (Vec::new(), None, false, false, DiagStyle::Full);
    while let Some(a) = it.next() {
        match a.as_str() {
            "-o" => out = Some(it.next()?),
            "--release" => release = true,
            "--report" => report = true,
            "--diagnostics=short" => style = DiagStyle::Short,
            s if s.starts_with('-') => return None,
            _ => positional.push(a),
        }
    }
    let [cmd, path]: [String; 2] = positional.try_into().ok()?;
    matches!(cmd.as_str(), "run" | "check" | "build").then_some(Args { cmd, path, out, release, report, style })
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
    if args.report && args.cmd != "check" {
        eprintln!("{REPORT_NEEDS_CHECK}");
        return ExitCode::from(2);
    }
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
    let (compiled, diagnostics) = kestrel_driver::compile(&args.path, src, args.style);
    eprint!("{diagnostics}");
    let Some(mut compiled) = compiled else { return ExitCode::from(1) };
    // An annotated program is analysed here, once, for this build; only `check` warns.
    let checking = args.cmd == "check";
    let proved = kestrel_driver::prove(&mut compiled, !args.release, checking, args.style);
    eprint!("{}", proved.text);
    if checking {
        // The report still prints when a contract fails: it is how to see why.
        if args.report {
            print_report(compiled, !args.release, proved.analysis);
        }
        return if proved.ok { ExitCode::SUCCESS } else { ExitCode::from(1) };
    }
    if !proved.ok {
        return ExitCode::from(1);
    }
    let opts = Options {
        checked_arith: !args.release,
        verify_proofs: proofs_checked(std::env::var_os("KESTREL_CHECK_PROOFS").as_deref()),
        ..Options::default()
    };
    let result = match args.cmd.as_str() {
        "run" if !has_sse41() => Err(NO_SSE41.to_string()),
        "run" => {
            // The program's relative paths (`window.screenshot`, textures) are read from its own folder.
            kestrel_window::set_program_dir(Path::new(&args.path).parent().unwrap_or(Path::new("")));
            JitProgram::compile_with(&compiled.module, opts, &kestrel_window::symbols())
                .map_err(internal_error)
                .map(|p| p.run_main())
        }
        _ => {
            // Programs that use `window`, `draw`, `draw3d` or `input` need the window library, which
            // carries the runtime as well.
            let lib = if compiled.builtins.is_empty() { Lib::Runtime } else { Lib::Window };
            build(&compiled.module, opts, &args, lib).map(|()| 0)
        }
    };
    match result {
        Ok(code) => ExitCode::from(code as u8),
        Err(e) => {
            eprintln!("kestrel: {e}");
            ExitCode::from(3)
        }
    }
}

/// Whether `KESTREL_CHECK_PROOFS=1` is set: codegen then keeps every check range analysis proved,
/// and a failing one stops the program with a message that names the compiler bug.
fn proofs_checked(value: Option<&std::ffi::OsStr>) -> bool {
    value.is_some_and(|v| v == "1")
}

/// Prints what range analysis leaves in the program for `kestrel check --report`, in the checks
/// of a checked build or of a `--release` build (`checked`). `analysis` is the driver's, of the
/// module as it is now, when it analysed it already (for that build).
fn print_report(mut compiled: kestrel_driver::Compiled, checked: bool, analysis: Option<kestrel_kir::range::Analysis>) {
    use kestrel_kir::range::{remove_checks, render::render, report::build, Arith};
    let analysis = analysis.unwrap_or_else(|| remove_checks(&mut compiled.module, Arith::of(checked)));
    let report = build(&compiled.module, &analysis);
    let files = &compiled.files;
    let text = render(&report, &compiled.module, &|i| files.get(i as usize).is_none_or(|f| !f.is_builtin()));
    print!("{text}");
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

fn build(module: &kestrel_kir::Module, opts: Options, args: &Args, lib: Lib) -> Result<(), String> {
    let exe = output_path(args)?;
    let bytes = emit_object(module, opts).map_err(internal_error)?;
    let obj = std::env::temp_dir().join(format!("kestrel-{}.o", std::process::id()));
    std::fs::write(&obj, bytes).map_err(|e| format!("can't write {}: {e}", obj.display()))?;
    let linked = link::link(&obj, &exe, lib);
    let _ = std::fs::remove_file(&obj);
    linked?;
    println!("built {} for {TARGET_CPUS}", exe.display());
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::process::Command;

    use super::{output_path, parse_args, proofs_checked, without_bom};

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
    fn the_proof_check_is_on_only_for_one() {
        assert!(proofs_checked(Some("1".as_ref())));
        assert!(!proofs_checked(Some("0".as_ref())));
        assert!(!proofs_checked(Some("".as_ref())));
        assert!(!proofs_checked(None));
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
