//! Helpers shared by the range-analysis tests: a program parsed, checked, lowered, optimised and
//! analysed, and the checks proven and left in one of its functions.
#![allow(dead_code)]

use kestrel_kir::range::{remove_checks, Analysis, Arith, CheckKind, CheckLeft, FuncChecks};
use kestrel_kir::*;
use kestrel_syntax::{parse, LineIndex};

/// `src` (which needs no `main`) optimised as the driver does, before range analysis.
pub fn optimized(src: &str) -> Module {
    let src = format!("{src}\nmain() {{}}\n");
    let (p, d) = parse(&src);
    assert!(d.is_empty(), "parse: {:?}", d.iter().map(|d| &d.message).collect::<Vec<_>>());
    let (t, d) = kestrel_check::check(&p);
    assert!(d.is_empty(), "check: {:?}", d.iter().map(|d| &d.message).collect::<Vec<_>>());
    let mut m = lower(&p, &t.unwrap(), &LineIndex::new(&src));
    optimize(&mut m);
    m
}

/// `src` optimised and analysed in `arith`.
pub fn analysed(src: &str, arith: Arith) -> (Module, Analysis) {
    let mut m = optimized(src);
    let a = remove_checks(&mut m, arith);
    (m, a)
}

pub fn idx(m: &Module, name: &str) -> usize {
    m.funcs.iter().position(|f| f.name == name).unwrap_or_else(|| panic!("no function `{name}`"))
}

pub fn checks<'a>(m: &Module, a: &'a Analysis, name: &str) -> &'a FuncChecks {
    &a.funcs[idx(m, name)]
}

/// The kind of check a proven node had.
fn proven_kind(inner: &Expr) -> CheckKind {
    match &inner.kind {
        ExprKind::Binary(BinOp::Div | BinOp::Rem, ..) => CheckKind::Division,
        ExprKind::Binary(BinOp::Shl | BinOp::Shr, ..) => CheckKind::Shift,
        ExprKind::Cast(_) => CheckKind::Narrowing,
        _ => CheckKind::Overflow,
    }
}

/// The kinds of the checks marked proven in `name`'s body (arithmetic nodes, then indexes).
pub fn proven_kinds(m: &Module, name: &str) -> Vec<CheckKind> {
    let mut out = Vec::new();
    visit::walk(&m.funcs[idx(m, name)].body, &mut |n| match n {
        visit::Node::Expr(Expr { kind: ExprKind::Proven(inner), .. }) => out.push(proven_kind(inner)),
        visit::Node::Place(p, _) => {
            for proj in &p.path {
                if matches!(proj, Proj::Index { check: IndexCheck::Proven, .. }) {
                    out.push(CheckKind::Bounds);
                }
            }
        }
        _ => {}
    });
    out
}

/// (checks proven, checks left) in `name`. The proven count is read from the marked module and
/// must match what the analysis says it removed.
pub fn counts(m: &Module, a: &Analysis, name: &str) -> (usize, usize) {
    let c = checks(m, a, name);
    let proven = proven_kinds(m, name).len();
    assert_eq!(proven, c.removed as usize, "`{name}`: marked nodes and the removed count differ");
    (proven, c.left.len())
}

/// The checks of `kind` left in `name`.
pub fn left_of<'a>(m: &Module, a: &'a Analysis, name: &str, kind: CheckKind) -> Vec<&'a CheckLeft> {
    checks(m, a, name).left.iter().filter(|c| c.kind == kind).collect()
}

/// How many checks of `kind` are proven in `name`.
pub fn proven_of(m: &Module, name: &str, kind: CheckKind) -> usize {
    proven_kinds(m, name).into_iter().filter(|&k| k == kind).count()
}

/// The source line (within its file) of a packed line.
pub fn line(packed: u32) -> u32 {
    unpack_line(packed).0
}
