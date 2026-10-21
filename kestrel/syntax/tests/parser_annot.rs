//! Parsing `@prove` and `@assume`.

use kestrel_syntax::ast::*;
use kestrel_syntax::Span;

mod common;
use common::*;

fn contracts(a: &ProveAnnot) -> Vec<Contract> {
    a.contracts.iter().map(|(c, _)| *c).collect()
}

fn iface_method(p: &Program, i: usize) -> &FnDecl {
    match &p.items[i] {
        Item::Interface(d) => &d.methods[0],
        other => panic!("expected an interface, got {other:?}"),
    }
}

fn stmt_span(body: &Block, i: usize) -> Span {
    body.stmts[i].span()
}

#[test]
fn prove_before_a_function_is_recorded() {
    let p = ok("@prove(no_alloc, no_checks)\npure step(x: int) -> int { x }\n");
    assert_eq!(p.proves.len(), 1);
    assert_eq!(p.proves[0].target, fn_decl(&p, 0).span);
    assert_eq!(contracts(&p.proves[0]), [Contract::NoAlloc, Contract::NoChecks]);
}

#[test]
fn prove_goes_before_pub_methods_and_interface_methods() {
    let src = "@prove(inline)\npub f() {}\n\
        struct S { x: int\n @prove(inline)\n get(self) -> int { self.x } }\n\
        enum E { A\n @prove(inline)\n id(self) -> int { 1 } }\n\
        interface I {\n @prove(inline)\n area(self) -> int\n }\n";
    let p = ok(src);
    let Item::Struct(s) = &p.items[1] else { panic!() };
    let Item::Enum(e) = &p.items[2] else { panic!() };
    let targets = [fn_decl(&p, 0).span, s.methods[0].span, e.methods[0].span, iface_method(&p, 3).span];
    assert_eq!(p.proves.len(), 4);
    for t in targets {
        assert!(p.proves.iter().any(|a| a.target == t && contracts(a) == [Contract::Inline]), "{t:?}");
    }
}

#[test]
fn prove_goes_before_loops() {
    let p = ok("main() {\n@prove(no_checks)\nfor i from 0 to 3 { }\n@prove(no_alloc)\npar for x in xs { }\n@prove(no_panic)\nwhile true { }\n}\n");
    let body = &fn_decl(&p, 0).body;
    assert_eq!(p.proves.len(), 3);
    for (i, c) in [Contract::NoChecks, Contract::NoAlloc, Contract::NoPanic].into_iter().enumerate() {
        assert_eq!((p.proves[i].target, contracts(&p.proves[i])), (stmt_span(body, i), vec![c]));
    }
    assert!(matches!(body.stmts[1], Stmt::ForIn { par: true, .. }));
}

#[test]
fn annotations_stack_and_merge() {
    for src in ["@prove(no_alloc) @prove(inline)\nf() {}", "@prove(no_alloc)\n@prove(inline)\nf() {}", "@prove(no_alloc, inline, no_alloc)\nf() {}"] {
        let p = ok(src);
        assert_eq!(p.proves.len(), 1, "{src}");
        assert_eq!(contracts(&p.proves[0]), [Contract::NoAlloc, Contract::Inline], "{src}");
        assert_eq!(p.proves[0].target, fn_decl(&p, 0).span);
    }
}

#[test]
fn the_target_may_follow_on_the_same_line() {
    let p = ok("@prove(inline) pure f() {}\nmain() { @prove(no_checks) for i from 0 to 3 {} }\n");
    assert_eq!(p.proves.len(), 2);
}

#[test]
fn a_blank_line_before_the_target_is_an_error() {
    let msg = "`@prove` goes on the line right before what it applies to, or on the same line";
    assert_eq!(errs("@prove(inline)\n\nf() {}\n"), [msg]);
    assert_eq!(errs("main() {\n@prove(inline)\n\nfor i from 0 to 3 {}\n}\n"), [msg]);
}

#[test]
fn prove_on_a_native_is_an_error() {
    assert_eq!(errs("@prove(no_alloc)\nnative f()\n"), ["`@prove` can't go on a native function"]);
}

#[test]
fn assume_is_a_statement() {
    let p = ok("f(n: int, xs: [int]) { @assume(n   <=\n xs.len) }\n");
    let body = &fn_decl(&p, 0).body;
    assert_eq!(body.to_sexpr(), "{(assume (<= n (. xs len)))}");
    let Stmt::Assume { text, .. } = &body.stmts[0] else { panic!() };
    assert_eq!(text, "n <= xs.len");
    assert_eq!(body.stmts[0].span().end - body.stmts[0].span().start, "@assume(n   <=\n xs.len)".len() as u32);
}

#[test]
fn two_assumes_share_a_line() {
    assert_eq!(body("@assume(a > 0) @assume(b > 0)"), "{(assume (> a 0)); (assume (> b 0))}");
    assert_eq!(body("@assume(a > 0) @prove(no_checks) while a < 3 { }"), "{(assume (> a 0)); (while (< a 3) {})}");
}

#[test]
fn assume_text_is_the_condition_as_written_in_a_later_file() {
    // `parse_at` shifts spans: the text must still come from the right place.
    let (p, d) = kestrel_syntax::parse_at("// pad\nf(n: int) { @assume(n > 0) }\n", 100, 0, 0);
    assert!(d.is_empty());
    let Item::Fn(f) = &p.items[0] else { panic!() };
    let Stmt::Assume { text, .. } = &f.body.stmts[0] else { panic!() };
    assert_eq!(text, "n > 0");
}

#[test]
fn annotations_are_found_in_every_kind_of_block() {
    let src = "f(n: int) {\n if n > 0 { @assume(n < 9) }\n arena { @assume(n < 9) }\n let g = (x: int) -> int { @assume(x < 9)\n x }\n}\n";
    ok(src);
}
