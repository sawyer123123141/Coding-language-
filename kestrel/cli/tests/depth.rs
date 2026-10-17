//! Code nested deeper than 1,000 levels is refused by the parser with one error, so no later
//! pass recurses over a deeper tree (50,000 parentheses or a 100,000-term sum used to overflow
//! the compiler's stack). The inputs are generated here rather than checked in.

use std::fs;
use std::path::PathBuf;
use std::process::Command;

mod common;

use common::hermetic;

const TOO_DEEP: &str = "this expression nests more than 1,000 levels deep; split it into smaller parts with `let`";
const TYPE_TOO_DEEP: &str = "this type nests more than 1,000 levels deep; simplify it";

/// Writes `src` to a file of its own and runs `kestrel check` on it: the exit code and the
/// `LINE: MESSAGE` of each error.
fn check(name: &str, src: &str) -> (Option<i32>, Vec<String>) {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("depth");
    fs::create_dir_all(&dir).unwrap();
    let path = dir.join(format!("{name}.kes"));
    fs::write(&path, src).unwrap();
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_kestrel"));
    hermetic(&mut cmd).args(["check", path.to_str().unwrap(), "--diagnostics=short"]);
    let out = cmd.output().expect("run kestrel");
    let stderr = String::from_utf8_lossy(&out.stderr);
    let errors = stderr
        .lines()
        .filter_map(|l| {
            let (pos, msg) = l.split_once(": error: ")?;
            Some(format!("{}: {msg}", pos.split(':').next()?))
        })
        .collect();
    (out.status.code(), errors)
}

/// `check` expects one error, `msg` on `line`.
fn refused(name: &str, src: &str, line: u32, msg: &str) {
    let (code, errors) = check(name, src);
    assert_eq!((code, errors), (Some(1), vec![format!("{line}: {msg}")]), "{name}");
}

fn accepted(name: &str, src: &str) {
    let (code, errors) = check(name, src);
    assert_eq!((code, errors), (Some(0), Vec::<String>::new()), "{name}");
}

/// `main` printing `expr`, which is on line 2.
fn printed(expr: &str) -> String {
    format!("main() {{\n    print({expr})\n}}\n")
}

/// `n` terms joined by `op`.
fn chain(term: &str, op: &str, n: usize) -> String {
    vec![term; n].join(op)
}

fn parens(n: usize, inner: &str) -> String {
    format!("{}{inner}{}", "(".repeat(n), ")".repeat(n))
}

#[test]
fn huge_nesting_is_one_error_not_a_crash() {
    refused("parens_50000", &printed(&parens(50_000, "1")), 2, TOO_DEEP);
    refused("sum_100000", &printed(&chain("1", " + ", 100_000)), 2, TOO_DEEP);
}

#[test]
fn the_limit_is_1000_levels() {
    // `print(` is one level, so 999 more parentheses reach 1,000; a `let` value starts at none.
    accepted("parens_1000", &printed(&parens(999, "1")));
    refused("parens_1001", &printed(&parens(1000, "1")), 2, TOO_DEEP);
    accepted("let_parens_1000", &format!("main() {{\n    let x = {}\n    print(x)\n}}\n", parens(1000, "1")));
    // A sum of 1,000 terms is 999 levels of `+`, inside `print(`.
    accepted("sum_1000", &printed(&chain("1", " + ", 1000)));
    refused("sum_1001", &printed(&chain("1", " + ", 1001)), 2, TOO_DEEP);
}

#[test]
fn every_kind_of_nesting_counts() {
    let deep = 5_000;
    let cases = [
        ("unary", format!("{}1", "- ".repeat(deep))),
        ("not", format!("{}true", "not ".repeat(deep))),
        ("product", chain("2", " * ", deep)),
        ("and", chain("true", " and ", deep)),
        ("bits", chain("1", " | ", deep)),
        ("calls", format!("{}1{}", "abs(".repeat(deep), ")".repeat(deep))),
        ("arrays", format!("{}1{}", "[".repeat(deep), "]".repeat(deep))),
        ("maps", format!("{}1{}", "{1: ".repeat(deep), "}".repeat(deep))),
        ("fields", format!("vec2(1, 2){}", ".x".repeat(deep))),
        ("methods", format!("\"a\"{}", ".len()".repeat(deep))),
        ("indexes", format!("[1]{}", "[0]".repeat(deep))),
        ("ifs", format!("{}1{}", "if true { ".repeat(deep), " } else { 0 }".repeat(deep))),
        ("else_ifs", format!("if false {{ 0 }}{} else {{ 1 }}", " else if false { 0 }".repeat(deep))),
        ("matches", format!("{}1{}", "match x { A { ".repeat(deep), " } else { 0 } }".repeat(deep))),
        ("interpolation", format!("\"{{{}}}\"", parens(deep, "1"))),
    ];
    for (name, expr) in cases {
        refused(name, &printed(&expr), 2, TOO_DEEP);
    }
    // An `if` used in a sum: the sum goes on top of what the `if` holds (989 levels of `+`).
    let nested_if = format!("if true {{ {} }} else {{ 0 }}", chain("1", " + ", 990));
    refused("sum_of_if", &printed(&format!("{nested_if}{}", " + 1".repeat(20))), 2, TOO_DEEP);
    // A sum whose first term is a sum in parentheses, again and again.
    let mut sums = "1".to_string();
    for _ in 0..40 {
        sums = format!("({sums}{})", " + 1".repeat(40));
    }
    refused("sums_of_sums", &printed(&sums), 2, TOO_DEEP);
}

#[test]
fn deep_blocks_report_the_line_where_the_limit_passes() {
    // `main`'s body is not a level; each `while` body is one.
    let blocks = |n: usize| format!("main() {{\n{}{}}}\n", "while true {\n".repeat(n), "}\n".repeat(n));
    accepted("blocks_1000", &blocks(1000));
    // The 1,001st `while` is on line 1,002.
    refused("blocks_5000", &blocks(5000), 1002, TOO_DEEP);
}

#[test]
fn deep_types_have_their_own_message() {
    let arrays = |n: usize| format!("main() {{\n    let x: {}int{} = []\n}}\n", "[".repeat(n), "]".repeat(n));
    refused("types_50000", &arrays(50_000), 2, TYPE_TOO_DEEP);
    refused("lists_5000", &format!("f(x: {}int{}) {{}}\nmain() {{}}\n", "list[".repeat(5000), "]".repeat(5000)), 1, TYPE_TOO_DEEP);
    refused("types_1001", &arrays(1001), 2, TYPE_TOO_DEEP);
    let (_, errors) = check("types_1000", &arrays(1000));
    assert!(!errors.iter().any(|e| e.contains("nests more than")), "{errors:?}");
}
