//! Lowering reads every checker type and resolution through `lower/subst.rs`, which applies the
//! substitution of the copy (or field default) being lowered (P19). A direct read of the tables
//! elsewhere in `lower/` would see a generic's own placeholders in a copy.

use std::path::{Path, PathBuf};

const FORBIDDEN: [&str; 14] = [
    "t.ty(",
    "t.res.get(",
    "t.res[",
    "t.coerce.get(",
    "iface_borrows.contains(",
    "t.sig(",
    "t.local(",
    "t.closures[",
    // The fields behind those accessors, read directly.
    "t.expr_ty[",
    "t.locals[",
    "t.sigs[",
    "t.closures.get(",
    "t.res.contains_key(",
    "t.coerce.contains_key(",
];

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).expect("lower/ is readable") {
        let path = entry.expect("a directory entry").path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

#[test]
fn lowering_reads_types_only_through_subst() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src").join("lower");
    let mut files = Vec::new();
    rust_files(&dir, &mut files);
    assert!(files.len() > 10, "found the lowering sources");
    let mut found = Vec::new();
    for path in files.iter().filter(|p| p.file_name().is_some_and(|n| n != "subst.rs")) {
        let text = std::fs::read_to_string(path).expect("a source file is readable");
        for (i, line) in text.lines().enumerate() {
            if line.trim_start().starts_with("//") {
                continue;
            }
            for pattern in FORBIDDEN.iter().filter(|p| line.contains(*p)) {
                let name = path.strip_prefix(&dir).unwrap_or(path).display();
                found.push(format!("{name}:{}: `{pattern}` in `{}`", i + 1, line.trim()));
            }
        }
    }
    assert!(found.is_empty(), "read these through `lower/subst.rs`:\n{}", found.join("\n"));
}
