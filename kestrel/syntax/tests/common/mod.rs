//! Helpers shared by the parser test binaries.
#![allow(dead_code)]

use kestrel_syntax::ast::*;
use kestrel_syntax::parse;

pub fn ok(src: &str) -> Program {
    let (p, d) = parse(src);
    let msgs: Vec<&String> = d.iter().map(|d| &d.message).collect();
    assert!(d.is_empty(), "unexpected diagnostics: {msgs:?}");
    p
}

pub fn errs(src: &str) -> Vec<String> {
    parse(src).1.into_iter().map(|d| d.message).collect()
}

pub fn fn_decl(p: &Program, i: usize) -> &FnDecl {
    match &p.items[i] {
        Item::Fn(f) => f,
        other => panic!("expected fn, got {other:?}"),
    }
}

/// Parses `src` as the body of `main` and returns the body as an s-expression.
pub fn body(src: &str) -> String {
    let p = ok(&format!("main() {{\n{src}\n}}"));
    fn_decl(&p, 0).body.to_sexpr()
}

/// Parses `src` (declarations and a `main`) with `parse`, which settles brackets, asserts there
/// are no diagnostics, and returns the body of the function named `main` as an s-expression.
pub fn main_body(src: &str) -> String {
    let p = ok(src);
    p.items
        .iter()
        .find_map(|i| match i {
            Item::Fn(f) if f.name == "main" => Some(f.body.to_sexpr()),
            _ => None,
        })
        .expect("a function named main")
}
