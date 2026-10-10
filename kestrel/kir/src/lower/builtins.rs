//! Lowering of the global built-ins.

use kestrel_check::Builtin;
use kestrel_syntax::ast;

use super::FnLowerer;
use crate::ir::*;

impl<'a> FnLowerer<'a> {
    /// `PI` or `TAU`, as a float literal of the expression's type.
    pub(crate) fn builtin_constant(b: Builtin) -> ExprKind {
        match b {
            Builtin::Pi => ExprKind::Float(std::f64::consts::PI),
            Builtin::Tau => ExprKind::Float(std::f64::consts::TAU),
            other => unreachable!("{other:?} isn't a constant"),
        }
    }

    /// A call to a global built-in function.
    pub(crate) fn builtin_call(&mut self, b: Builtin, args: &[ast::Arg]) -> ExprKind {
        let op = match b {
            Builtin::Sqrt => MathOp::Sqrt,
            Builtin::Sin => MathOp::Sin,
            Builtin::Cos => MathOp::Cos,
            Builtin::Tan => MathOp::Tan,
            Builtin::Asin => MathOp::Asin,
            Builtin::Acos => MathOp::Acos,
            Builtin::Atan => MathOp::Atan,
            Builtin::Atan2 => MathOp::Atan2,
            Builtin::Exp => MathOp::Exp,
            Builtin::Log => MathOp::Log,
            Builtin::Pow => MathOp::Pow,
            Builtin::Floor => MathOp::Floor,
            Builtin::Ceil => MathOp::Ceil,
            Builtin::Round => MathOp::Round,
            other => unreachable!("the checker doesn't accept {other:?} yet"),
        };
        ExprKind::Math(op, args.iter().map(|a| self.expr(&a.value)).collect())
    }
}
