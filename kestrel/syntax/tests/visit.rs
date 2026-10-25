//! The expression visitor reaches every expression, closure bodies and interpolations included.

use kestrel_syntax::ast::*;
use kestrel_syntax::visit::{each_expr, each_expr_in};

mod common;
use common::{fn_decl, ok};

#[test]
fn each_expr_reaches_closures_assumptions_and_interpolations() {
    let p = ok("main() {\n    let f = (x: int) { x + 1 }\n    @assume(n > 0)\n    print(\"a {f(2)}\")\n}\n");
    let mut seen = Vec::new();
    each_expr(&fn_decl(&p, 0).body, &mut |e| seen.push(e.to_sexpr()));
    for want in ["(+ x 1)", "(> n 0)", "(call f 2)"] {
        assert!(seen.iter().any(|s| s == want), "missing {want} in {seen:?}");
    }
}

#[test]
fn a_block_visits_its_own_expression_before_its_parts() {
    let p = ok("main() {\n    let a = (1 + 2) * 3\n}\n");
    let Stmt::Let { value, .. } = &fn_decl(&p, 0).body.stmts[0] else { panic!("a let") };
    let mut seen = Vec::new();
    each_expr_in(value, &mut |e| seen.push(e.to_sexpr()));
    assert_eq!(seen, ["(* (+ 1 2) 3)", "(+ 1 2)", "1", "2", "3"]);
}

#[test]
fn if_match_and_let_else_blocks_are_walked() {
    let p = ok("main() {\n    let v = opt else { print(7) }\n    if c { print(1) } else { print(2) }\n}\n");
    let mut seen = Vec::new();
    each_expr(&fn_decl(&p, 0).body, &mut |e| seen.push(e.to_sexpr()));
    for want in ["opt", "(call print 7)", "c", "(call print 1)", "(call print 2)"] {
        assert!(seen.iter().any(|s| s == want), "missing {want} in {seen:?}");
    }
}
