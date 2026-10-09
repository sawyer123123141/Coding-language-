use std::path::{Path, PathBuf};
use std::process::Command;

/// Where the runtime library was left when this compiler was built.
const BUILT_RUNTIME_LIB: &str = env!("KESTREL_RUNTIME_LIB");

const RUNTIME_LIB_FILE: &str = if cfg!(windows) { "kestrel_runtime.lib" } else { "libkestrel_runtime.a" };

/// Links an object file with the Kestrel runtime into an executable.
pub fn link(obj: &Path, exe: &Path) -> Result<(), String> {
    let runtime = runtime_lib()?;
    let mut cmd = linker()?;
    if cfg!(windows) {
        cmd.arg("/NOLOGO")
            .arg(format!("/OUT:{}", exe.display()))
            .arg(obj)
            .arg(&runtime)
            // What Rust's std needs (`rustc --print native-static-libs`).
            .args(["kernel32.lib", "ntdll.lib", "userenv.lib", "ws2_32.lib", "dbghelp.lib", "msvcrt.lib"])
            .args(["/NODEFAULTLIB:libcmt", "/SUBSYSTEM:CONSOLE"]);
    } else {
        cmd.arg(obj).arg(&runtime).arg("-o").arg(exe).args(["-lpthread", "-ldl", "-lm"]);
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

/// The runtime library: next to this executable, else where the build left it.
fn runtime_lib() -> Result<PathBuf, String> {
    let exe_dir = std::env::current_exe().ok().and_then(|exe| exe.parent().map(Path::to_path_buf));
    find_runtime_lib(exe_dir.as_deref(), Path::new(BUILT_RUNTIME_LIB))
}

fn find_runtime_lib(exe_dir: Option<&Path>, built: &Path) -> Result<PathBuf, String> {
    exe_dir
        .map(|dir| dir.join(RUNTIME_LIB_FILE))
        .filter(|lib| lib.is_file())
        .or_else(|| built.is_file().then(|| built.to_path_buf()))
        .ok_or_else(|| format!("can't find the Kestrel runtime library ({RUNTIME_LIB_FILE}); rebuild kestrel"))
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

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};

    use super::{find_runtime_lib, RUNTIME_LIB_FILE};

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("kestrel-link-test-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn library_next_to_the_executable_wins() {
        let (exe_dir, other) = (scratch("exe"), scratch("built"));
        fs::write(exe_dir.join(RUNTIME_LIB_FILE), b"a").unwrap();
        fs::write(other.join(RUNTIME_LIB_FILE), b"b").unwrap();
        let found = find_runtime_lib(Some(&exe_dir), &other.join(RUNTIME_LIB_FILE)).unwrap();
        assert_eq!(found, exe_dir.join(RUNTIME_LIB_FILE));
    }

    #[test]
    fn falls_back_to_the_built_path() {
        let (exe_dir, other) = (scratch("exe_empty"), scratch("built_only"));
        fs::write(other.join(RUNTIME_LIB_FILE), b"b").unwrap();
        let built = other.join(RUNTIME_LIB_FILE);
        assert_eq!(find_runtime_lib(Some(&exe_dir), &built).unwrap(), built);
        assert_eq!(find_runtime_lib(None, &built).unwrap(), built);
    }

    #[test]
    fn missing_library_is_a_clear_error() {
        let exe_dir = scratch("exe_none");
        let err = find_runtime_lib(Some(&exe_dir), Path::new("no/such/lib")).unwrap_err();
        assert_eq!(err, format!("can't find the Kestrel runtime library ({RUNTIME_LIB_FILE}); rebuild kestrel"));
    }
}
