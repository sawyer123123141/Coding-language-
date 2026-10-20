//! Which checks codegen emits for a KIR node: the sites range analysis decides. This mirrors
//! codegen's `func/arith.rs` and `func/divide.rs` exactly; a site missing here would never be
//! removed (sound), but one listed here that codegen doesn't emit would be counted wrongly.

use crate::ir::*;

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
        ExprKind::Unary(UnOp::Neg, _) if e.ty.is_int() && checked => vec![CheckKind::Overflow],
        ExprKind::Binary(op, l, r) if l.ty.is_int() => match op {
            BinOp::Add | BinOp::Sub | BinOp::Mul if checked => vec![CheckKind::Overflow],
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
