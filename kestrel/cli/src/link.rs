use std::path::Path;
use std::process::Command;

const RUNTIME_LIB: &str = env!("KESTREL_RUNTIME_LIB");

/// Links an object file with the Kestrel runtime into an executable.
pub fn link(obj: &Path, exe: &Path) -> Result<(), String> {
    let mut cmd = linker()?;
    if cfg!(windows) {
        cmd.arg("/NOLOGO")
            .arg(format!("/OUT:{}", exe.display()))
            .arg(obj)
            .arg(RUNTIME_LIB)
            // What Rust's std needs (`rustc --print native-static-libs`).
            .args(["kernel32.lib", "ntdll.lib", "userenv.lib", "ws2_32.lib", "dbghelp.lib", "msvcrt.lib"])
            .args(["/NODEFAULTLIB:libcmt", "/SUBSYSTEM:CONSOLE"]);
    } else {
        cmd.arg(obj).arg(RUNTIME_LIB).arg("-o").arg(exe).args(["-lpthread", "-ldl", "-lm"]);
    }
    let out = cmd.output().map_err(|e| format!("couldn't start the linker: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "linking failed:\n{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    Ok(())
}

#[cfg(windows)]
fn linker() -> Result<Command, String> {
    // Never a bare `link.exe` from PATH: Git for Windows ships an unrelated `link`.
    find_msvc_tools::find("x86_64-pc-windows-msvc", "link.exe")
        .ok_or_else(|| "couldn't find the Microsoft linker; install Visual Studio Build Tools (C++)".to_string())
}

#[cfg(not(windows))]
fn linker() -> Result<Command, String> {
    Ok(Command::new("cc"))
}
