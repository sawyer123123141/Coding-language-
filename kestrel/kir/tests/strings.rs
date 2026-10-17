use kestrel_kir::*;
use kestrel_syntax::{parse, LineIndex};

fn lower_src(src: &str) -> Module {
    let (p, d) = parse(src);
    assert!(d.is_empty(), "parse: {:?}", d.iter().map(|d| &d.message).collect::<Vec<_>>());
    let (t, d) = kestrel_check::check(&p);
    assert!(d.is_empty(), "check: {:?}", d.iter().map(|d| &d.message).collect::<Vec<_>>());
    lower(&p, &t.unwrap(), &LineIndex::new(src))
}

fn main_body(m: &Module) -> &[Stmt] {
    &m.funcs.iter().find(|f| f.name == "main").unwrap().body
}

/// Every expression in `stmts`, innermost included.
fn exprs(stmts: &[Stmt]) -> Vec<&Expr> {
    let mut out = Vec::new();
    visit::walk(stmts, &mut |n| {
        if let visit::Node::Expr(e) = n {
            out.push(e);
        }
    });
    out
}

fn dups(stmts: &[Stmt]) -> usize {
    let mut n = 0;
    visit::walk(stmts, &mut |node| {
        if let visit::Node::Stmt(Stmt::Dup(_)) = node {
            n += 1;
        }
    });
    n
}

#[test]
fn length_and_index_are_string_operations_not_places() {
    let m = lower_src("main() {\n let s = \"héllo\"\n let n = s.len\n let c = s[1]\n}\n");
    let ops: Vec<StrOp> = exprs(main_body(&m)).into_iter().filter_map(|e| if let ExprKind::Str(op, _) = &e.kind { Some(*op) } else { None }).collect();
    assert_eq!(ops, [StrOp::Chars, StrOp::At]);
    // No `Index` step into a string, and no bytes length either.
    let walked = exprs(main_body(&m));
    assert!(!walked.iter().any(|e| matches!(&e.kind, ExprKind::Len(_))));
    assert!(!walked.iter().any(|e| matches!(&e.kind, ExprKind::Load(p) if p.path.iter().any(|s| matches!(s, Proj::Index { .. })))));
}

#[test]
fn a_chain_of_plus_is_one_interpolation() {
    let m = lower_src("main() {\n let a = \"a\"\n let b = a + \"-\" + a + \"!\"\n}\n");
    let [.., Stmt::Let(_, b)] = main_body(&m) else { panic!("{:#?}", main_body(&m)) };
    let ExprKind::Format(parts) = &b.kind else { panic!("{b:?}") };
    assert_eq!(parts.len(), 4, "{parts:?}");
    assert!(matches!(&parts[1], PrintArg::Str(s) if s == "-"));
}

#[test]
fn append_builds_a_new_string_from_the_old_one() {
    let m = lower_src("main() {\n var s = \"a\"\n s += \"b\"\n}\n");
    let [_, Stmt::Assign(_, v)] = main_body(&m) else { panic!("{:#?}", main_body(&m)) };
    let ExprKind::Format(parts) = &v.kind else { panic!("{v:?}") };
    assert!(matches!(&parts[0], PrintArg::Value(Expr { kind: ExprKind::Load(_), .. })));
    assert!(matches!(&parts[1], PrintArg::Str(s) if s == "b"));
}

#[test]
fn a_string_loop_walks_a_snapshot_and_copies_it_only_when_the_body_may_change_the_string() {
    let steady = lower_src("main() {\n let s = \"ab\"\n var n = 0\n for c in s { n += 1 }\n}\n");
    assert_eq!(dups(main_body(&steady)), 0);
    let ops: Vec<StrOp> =
        exprs(main_body(&steady)).into_iter().filter_map(|e| if let ExprKind::Str(op, _) = &e.kind { Some(*op) } else { None }).collect();
    assert_eq!(ops, [StrOp::CharLen, StrOp::Piece]);
    let changing = lower_src("main() {\n var s = \"ab\"\n for c in s { s = \"x\" }\n}\n");
    assert_eq!(dups(main_body(&changing)), 1);
}

#[test]
fn the_loop_steps_before_the_body_so_continue_lands_past_the_character() {
    let m = lower_src("main() {\n var n = 0\n for i, c in \"abc\" {\n if i == 1 { continue }\n n += 1\n }\n}\n");
    let body = main_body(&m);
    let Some(Stmt::While { body: pass, .. }) = body.iter().find(|s| matches!(s, Stmt::While { .. })) else { panic!("{body:#?}") };
    // CharLen, Piece, step pos, bind i, step k, then the user's statements.
    assert!(matches!(&pass[0], Stmt::Let(_, Expr { kind: ExprKind::Str(StrOp::CharLen, _), .. })));
    assert!(matches!(&pass[1], Stmt::Let(_, Expr { kind: ExprKind::Str(StrOp::Piece, _), .. })));
    assert!(matches!(&pass[2], Stmt::Assign(..)));
    assert!(matches!(&pass[5], Stmt::If { .. }), "{pass:#?}");
}
