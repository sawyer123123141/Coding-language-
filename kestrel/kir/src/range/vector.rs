//! Vectors in range analysis: the forms that make and take apart a SIMD vector (`VecLit`, `Splat`,
//! `Shuffle`, `LaneOf`), the checked `ivec4` operations, and how the report writes them. A float
//! vector has no checks and isn't analysed; a vector's value is any value of its lane type (an int
//! vector's one range for all four lanes is Task 4's), and every interval helper is asked about the
//! lane type. A vector never has a linear form (M13), and for now every `ivec4` check is kept: each
//! is decided, not proven, so the report and the contracts count it.

use super::bounds::Val;
use super::interval::Interval;
use super::show::{expr_text, range_text, write_expr, write_operand};
use super::sites::CheckKind;
use super::state::State;
use super::Cx;
use crate::ir::*;

/// The lane names, in order.
const LANES: [&str; 4] = ["x", "y", "z", "w"];

/// A vector type's name as Kestrel writes it.
pub fn vector_name(ty: KTy) -> &'static str {
    match ty {
        KTy::I32X4 => "ivec4",
        _ => "vec4",
    }
}

/// The name of lane `i`: `x`, `y`, `z` or `w`.
pub fn lane_name(i: u8) -> &'static str {
    LANES[i as usize]
}

impl Cx<'_> {
    /// A vector form: its parts evaluated (deciding their checks), and its value any value of
    /// its lane type.
    pub(super) fn vector(&mut self, st: &State, e: &Expr, sure: bool) -> Val {
        match &e.kind {
            ExprKind::VecLit(xs) => self.all(st, xs, sure),
            ExprKind::Splat(x) | ExprKind::Shuffle(x, _) | ExprKind::LaneOf(x, _) => {
                self.eval(st, x, sure);
            }
            _ => unreachable!("not a vector form"),
        }
        Val::range(Interval::of_type(e.ty.lane()))
    }

    /// An `ivec4` operation (`is_int_vector_op`): its operands evaluated, and its check, if this
    /// build has one, decided as not proven (unless `already` proven). Its value is any `int` in
    /// every lane, with no linear form; a comparison's is any `bool`.
    pub(super) fn vector_arith(&mut self, st: &State, e: &Expr, sure: bool, already: bool) -> Val {
        let any = Val::range(Interval::of_type(e.ty.lane()));
        match &e.kind {
            ExprKind::Unary(UnOp::Neg, x) => {
                let a = self.eval(st, x, sure);
                self.site(e, CheckKind::Overflow, false, already, &[(x, a)]);
            }
            ExprKind::Binary(op, l, r) => {
                let (a, b) = (self.eval(st, l, sure), self.eval(st, r, sure));
                let kind = match op {
                    BinOp::Add | BinOp::Sub | BinOp::Mul => CheckKind::Overflow,
                    BinOp::Shl | BinOp::Shr => CheckKind::Shift,
                    _ => return any,
                };
                self.site(e, kind, false, already, &[(l, a), (r, b)]);
            }
            _ => unreachable!("not an `ivec4` operation"),
        }
        any
    }
}

/// Whether `e` is an `ivec4` operation that arithmetic analysis sends here: `-v` or any binary
/// operation on `ivec4`s (a shift's amount is a plain `int`).
pub(super) fn is_int_vector_op(e: &Expr) -> bool {
    match &e.kind {
        ExprKind::Unary(UnOp::Neg, _) => e.ty == KTy::I32X4,
        ExprKind::Binary(_, l, _) => l.ty == KTy::I32X4,
        _ => false,
    }
}

/// The evidence for an operand `x` of a vector check whose range is `r`: "the lanes of `a` are 0 to
/// 99" for a vector (one range for its four lanes), the scalar phrase for a shift's amount.
pub(super) fn lanes_evidence(m: &Module, f: &Func, x: &Expr, r: Interval) -> String {
    let text = expr_text(m, f, x);
    if !x.ty.is_vector() {
        return format!("`{text}` {}", range_text(r, x.ty));
    }
    let phrase = range_text(r, x.ty.lane());
    let phrase = match phrase.strip_prefix("is ") {
        Some(rest) => format!("are {rest}"),
        None => phrase,
    };
    format!("the lanes of `{text}` {phrase}")
}

/// Whether `x` is a literal as an operand of a check: a number, or a number filling every lane.
pub(super) fn is_literal(x: &Expr) -> bool {
    match &x.kind {
        ExprKind::Int(_) => true,
        ExprKind::Splat(n) => matches!(n.kind, ExprKind::Int(_)),
        _ => false,
    }
}

/// A vector form in Kestrel syntax: `vec4(a, b, c, d)`, `vec4(n)` for a `Splat`, `v.wzyx`, `e.x`.
pub(super) fn write_vector(m: &Module, f: &Func, e: &Expr, s: &mut String) {
    match &e.kind {
        ExprKind::VecLit(xs) => {
            s.push_str(vector_name(e.ty));
            s.push('(');
            for (i, x) in xs.iter().enumerate() {
                if i > 0 {
                    s.push_str(", ");
                }
                write_expr(m, f, x, s);
            }
            s.push(')');
        }
        ExprKind::Splat(x) => {
            s.push_str(vector_name(e.ty));
            s.push('(');
            write_expr(m, f, x, s);
            s.push(')');
        }
        ExprKind::Shuffle(x, lanes) => {
            write_operand(m, f, x, s);
            s.push('.');
            lanes.iter().for_each(|&i| s.push_str(lane_name(i)));
        }
        ExprKind::LaneOf(x, i) => {
            write_operand(m, f, x, s);
            s.push('.');
            s.push_str(lane_name(*i));
        }
        _ => unreachable!("not a vector form"),
    }
}
