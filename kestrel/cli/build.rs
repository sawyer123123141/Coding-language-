use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Builds kestrel-runtime as a staticlib (which `cargo test` doesn't do for
/// workspace members), exports its path as KESTREL_RUNTIME_LIB, and copies it
/// next to the `kestrel` binary where `kestrel build` looks first.
fn main() {
    let out = PathBuf::from(env::var("OUT_DIR").unwrap());
    let workspace = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap()).join("..");
    let target_dir = out.join("runtime-target");
    let release = env::var("PROFILE").unwrap() == "release";
    let windows = env::var("CARGO_CFG_TARGET_OS").unwrap() == "windows";

    let mut cmd = Command::new(env::var("CARGO").unwrap());
    cmd.current_dir(&workspace)
        .args(["build", "-p", "kestrel-runtime", "--lib", "--target-dir"])
        .arg(&target_dir);
    if release {
        cmd.arg("--release");
    }
    // Keep the outer build's settings (profile, features, target dir) out of the nested one.
    for (key, _) in env::vars() {
        if key.starts_with("CARGO_") && key != "CARGO_HOME" {
            cmd.env_remove(&key);
        }
    }
    cmd.env_remove("RUSTFLAGS");
    let status = cmd.status().expect("start cargo for kestrel-runtime");
    assert!(status.success(), "building kestrel-runtime failed");

    let file = if windows { "kestrel_runtime.lib" } else { "libkestrel_runtime.a" };
    let lib = target_dir.join(if release { "release" } else { "debug" }).join(file);
    assert!(lib.exists(), "runtime staticlib missing at {}", lib.display());
    println!("cargo:rustc-env=KESTREL_RUNTIME_LIB={}", lib.display());
    println!("cargo:rerun-if-changed=../runtime/src");
    println!("cargo:rerun-if-changed=../runtime/Cargo.toml");

    // Best effort: `kestrel build` falls back to the path above if this is skipped or fails.
    if let Some(dir) = binary_dir(&out) {
        if let Err(e) = install_beside(&lib, &dir.join(file)) {
            println!("cargo:warning=couldn't copy the runtime library next to the kestrel binary: {e}");
        }
    }
}

/// The directory the `kestrel` binary lands in, for an OUT_DIR shaped
/// `<dir>/build/<package>-<hash>/out`. Any other layout gives `None`, so we never guess.
fn binary_dir(out: &Path) -> Option<PathBuf> {
    let build = out.ancestors().nth(2)?;
    if build.file_name()? != "build" {
        return None;
    }
    build.parent().map(Path::to_path_buf)
}

/// Replaces `dest` with a copy of `lib`. Going through a temporary file keeps
/// cargo's own hard-linked output of the same name (and its `deps/` original) untouched.
fn install_beside(lib: &Path, dest: &Path) -> std::io::Result<()> {
    let tmp = dest.with_extension("tmp");
    fs::copy(lib, &tmp)?;
    fs::rename(&tmp, dest).inspect_err(|_| {
        let _ = fs::remove_file(&tmp);
    })
}
