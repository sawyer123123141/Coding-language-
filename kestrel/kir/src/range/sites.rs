//! Which checks codegen emits for a KIR node: the sites range analysis decides. This mirrors
//! codegen's `func/arith.rs`, `func/divide.rs` and `func/simd_check.rs` exactly; a site missing here would never be
//! removed (sound), but one listed here that codegen doesn't emit would be counted wrongly. Also
//! every site of a function, found by walking it (`sites_of`).

use super::{expr_key, index_key, show, CheckLeft, Key};
use crate::ir::*;
use crate::visit::{walk, Node};

/// The build's arithmetic: with checks on, or `--release` (overflow, shift and narrowing checks
/// off; operations wrap).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Arith {
    Checked,
    Wrapping,
}

impl Arith {
    /// The mode of a build whose `Options::checked_arith` is `checked_arith`.
    pub fn of(checked_arith: bool) -> Arith {
        if checked_arith {
            Arith::Checked
        } else {
            Arith::Wrapping
        }
    }

    pub fn checked(self) -> bool {
        self == Arith::Checked
    }
}

/// What a check guards against.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CheckKind {
    /// An integer `+ - *` or `-x` whose result doesn't fit its type.
    Overflow,
    /// A shift by a negative amount or by the width or more.
    Shift,
    /// A division or remainder by 0, or (signed) the minimum by -1.
    Division,
    /// A narrowing conversion of a value that doesn't fit.
    Narrowing,
    /// An index outside its list or array.
    Bounds,
}

/// A literal divisor as codegen writes it: wrapped to its type (`divide.rs`'s `wrap_literal`).
pub fn wrap_literal(ty: KTy, c: i64) -> i64 {
    match ty {
        KTy::U8 => c & 0xFF,
        KTy::I32 => c as i32 as i64,
        _ => c,
    }
}

/// The checks codegen emits for this node itself (not its operands), in `arith`. Bounds checks
/// belong to places: see `is_bounds_site`.
pub fn checks_of(e: &Expr, arith: Arith) -> Vec<CheckKind> {
    let checked = arith.checked();
    match &e.kind {
        ExprKind::Unary(UnOp::Neg, _) if (e.ty.is_int() || e.ty == KTy::I32X4) && checked => vec![CheckKind::Overflow],
        // An `ivec4` operation is one check for all four lanes (`func/simd_check.rs`); it has no
        // `/` or `%`.
        ExprKind::Binary(op, l, _) if l.ty == KTy::I32X4 => match op {
            BinOp::Add | BinOp::Sub | BinOp::Mul if checked => vec![CheckKind::Overflow],
            BinOp::Shl | BinOp::Shr if checked => vec![CheckKind::Shift],
            _ => Vec::new(),
        },
        ExprKind::Binary(op, l, r) if l.ty.is_int() => match op {
            BinOp::Add | BinOp::Sub | BinOp::Mul if checked => vec![CheckKind::Overflow],
            // `*% +% -%` wrap and never check.
            BinOp::AddWrap | BinOp::SubWrap | BinOp::MulWrap => Vec::new(),
            BinOp::Shl | BinOp::Shr if checked => vec![CheckKind::Shift],
            BinOp::Div | BinOp::Rem if divides_checked(l.ty, r) => vec![CheckKind::Division],
            _ => Vec::new(),
        },
        ExprKind::Cast(x) if checked && is_narrowing(x.ty, e.ty) => vec![CheckKind::Narrowing],
        _ => Vec::new(),
    }
}

/// Whether a division of a `ty` value by `divisor` has checks: any divisor that isn't a literal,
/// and a literal that wraps to 0 or (signed) -1. Any other literal divides without one.
pub fn divides_checked(ty: KTy, divisor: &Expr) -> bool {
    match divisor.kind {
        ExprKind::Int(c) => {
            let c = wrap_literal(ty, c);
            c == 0 || (ty.is_signed() && c == -1)
        }
        _ => true,
    }
}

/// Whether converting `from` to `to` is an integer narrowing, which codegen checks (both integer
/// types, the target narrower).
pub fn is_narrowing(from: KTy, to: KTy) -> bool {
    from.is_int() && to.is_int() && super::interval::width(to) < super::interval::width(from)
}

/// Whether an element access is bounds-checked: an index that is still `Checked`.
pub fn is_bounds_site(p: &Proj) -> bool {
    matches!(p, Proj::Index { check: IndexCheck::Checked, .. })
}

/// A check site found by walking a function: an arithmetic node's check, or step `i` of a place.
pub(super) enum Site<'a> {
    Expr(CheckKind, &'a Expr),
    Bounds(&'a Place, usize),
}

impl Site<'_> {
    /// The site as a check left, inside the `Prove` blocks `regions`, with the text of a check
    /// nothing is known about (with `texts`).
    pub(super) fn left(&self, m: &Module, f: &Func, texts: bool, regions: Vec<Region>) -> CheckLeft {
        match *self {
            Site::Expr(kind, e) => {
                let text = if texts { show::check_text(m, f, kind, e, &[]) } else { String::new() };
                CheckLeft { kind, line: e.line, loops: Vec::new(), regions, text }
            }
            Site::Bounds(p, i) => {
                let line = p.path[i].at().map_or(0, |at| at.line);
                let text = if texts { show::bounds_text(m, f, p, i, None) } else { String::new() };
                CheckLeft { kind: CheckKind::Bounds, line, loops: Vec::new(), regions, text }
            }
        }
    }
}

/// Every check site in `f`, in walk order, with the ids of the `Prove` blocks around it. The node
/// inside a `Proven` (a module analysed before) has no checks left, so it is no site.
pub(super) fn sites_of(f: &Func, arith: Arith) -> Vec<(Key, Site<'_>, Vec<Region>)> {
    let mut out = Vec::new();
    let mut inside_proven: Option<*const Expr> = None;
    let mut open = Regions::default();
    walk(&f.body, &mut |n| {
        let regions = open.now();
        open.step(&n);
        match n {
            Node::Expr(e) => {
                if let ExprKind::Proven(inner) = &e.kind {
                    inside_proven = Some(&**inner as *const Expr);
                }
                if inside_proven == Some(e as *const Expr) {
                    inside_proven = None;
                    return;
                }
                for kind in checks_of(e, arith) {
                    out.push((expr_key(e), Site::Expr(kind, e), regions.clone()));
                }
            }
            Node::Place(p, _) => {
                for (i, proj) in p.path.iter().enumerate() {
                    if is_bounds_site(proj) {
                        out.push((index_key(proj), Site::Bounds(p, i), regions.clone()));
                    }
                }
            }
            Node::Stmt(_) => {}
        }
    });
    out
}

/// The `Prove` blocks open at each node of a `visit::walk`, which reports a block's statement
/// before everything in it and has no end marker: each open block counts down the nodes left in
/// it, and closes when none are. Blocks are numbered in the walk's order (`Region::copy`).
#[derive(Default)]
struct Regions {
    open: Vec<(Region, usize)>,
    seen: u32,
}

impl Regions {
    fn now(&self) -> Vec<Region> {
        self.open.iter().map(|&(r, _)| r).collect()
    }

    /// Steps past node `n`, which was inside every block open before it.
    fn step(&mut self, n: &Node) {
        for (_, left) in &mut self.open {
            *left -= 1;
        }
        while self.open.last().is_some_and(|&(_, left)| left == 0) {
            self.open.pop();
        }
        if let Node::Stmt(Stmt::Prove { id, at, body }) = n {
            let region = Region { id: *id, at: *at, copy: self.seen };
            self.seen += 1;
            let mut inside = 0;
            walk(body, &mut |_| inside += 1);
            if inside > 0 {
                self.open.push((region, inside));
            }
        }
    }
}
