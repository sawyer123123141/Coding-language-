//! The passes that run before range analysis walk into a `Proven` expression, even though only
//! range analysis makes one (as KIR's last step): here one is put in by hand.

use kestrel_kir::visit_mut::{walk_expr, VisitMut};
use kestrel_kir::*;
use kestrel_syntax::{parse, LineIndex};

/// Wraps every integer multiplication in `Proven`.
struct Wrap;

impl VisitMut for Wrap {
    fn visit_expr(&mut self, e: &mut Expr) {
        walk_expr(self, e);
        if matches!(e.kind, ExprKind::Binary(BinOp::Mul, ..)) {
            let (ty, line) = (e.ty, e.line);
            let inner = std::mem::replace(e, Expr { kind: ExprKind::Bool(false), ty, line });
            *e = Expr { kind: ExprKind::Proven(Box::new(inner)), ty, line };
        }
    }
}

fn module() -> Module {
    let src = "pure sq(x: int) -> int { x * x }\nf(a: int) -> int { sq(a + 1) }\nmain() {}\n";
    let (p, d) = parse(src);
    assert!(d.is_empty(), "parse: {:?}", d.iter().map(|d| &d.message).collect::<Vec<_>>());
    let (t, d) = kestrel_check::check(&p);
    assert!(d.is_empty(), "check: {:?}", d.iter().map(|d| &d.message).collect::<Vec<_>>());
    lower(&p, &t.unwrap(), &LineIndex::new(src))
}

fn func<'m>(m: &'m Module, name: &str) -> &'m Func {
    m.funcs.iter().find(|f| f.name == name).unwrap()
}

/// How many local reads there are inside `Proven` nodes of `stmts`, and how many `Proven` nodes.
fn proven_reads(stmts: &[Stmt]) -> (usize, usize) {
    let (mut reads, mut nodes) = (0, 0);
    visit::walk(stmts, &mut |n| {
        if let visit::Node::Expr(Expr { kind: ExprKind::Proven(inner), .. }) = n {
            nodes += 1;
            visit::walk_expr(inner, &mut |m| {
                if matches!(m, visit::Node::Expr(Expr { kind: ExprKind::Load(_), .. })) {
                    reads += 1;
                }
            });
        }
    });
    (reads, nodes)
}

#[test]
fn the_walkers_and_the_inliner_go_into_a_proven_expression() {
    let mut m = module();
    for f in &mut m.funcs {
        Wrap.visit_stmts(&mut f.body);
    }
    assert_eq!(proven_reads(&func(&m, "sq").body), (2, 1), "the walker sees both reads of `x`");
    optimize(&mut m);
    // `f` has `sq` inlined: its body now holds the proven multiplication, renamed to f's locals.
    let (reads, nodes) = proven_reads(&func(&m, "f").body);
    assert_eq!(nodes, 1, "{:#?}", func(&m, "f").body);
    assert!(reads >= 1, "the inlined operands are still read inside the Proven node: {:#?}", func(&m, "f").body);
}
