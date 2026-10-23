//! Vectors in range analysis: the forms that make and take apart a SIMD vector (`VecLit`, `Splat`,
//! `Shuffle`, `LaneOf`) and how the report writes them. A float vector has no checks and isn't
//! analysed; a vector's value is any value of its lane type (an int vector's one range for all
//! four lanes is Tasks 3b and 4's), and every interval helper is asked about the lane type.

use super::bounds::Val;
use super::interval::Interval;
use super::show::{write_expr, write_operand};
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
