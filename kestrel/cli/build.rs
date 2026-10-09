use std::env;
use std::path::PathBuf;
use std::process::Command;

/// Builds kestrel-runtime as a staticlib (which `cargo test` doesn't do for
/// workspace members) and exports its path as KESTREL_RUNTIME_LIB.
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
}
