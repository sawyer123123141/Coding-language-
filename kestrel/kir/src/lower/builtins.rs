//! Lowering of the global built-ins.

use kestrel_check::Builtin;
use kestrel_syntax::ast;

use super::FnLowerer;
use crate::ir::*;

/// The value of `PI` or `TAU`; `None` for any other built-in.
pub(crate) fn constant_value(b: Builtin) -> Option<f64> {
    match b {
        Builtin::Pi => Some(std::f64::consts::PI),
        Builtin::Tau => Some(std::f64::consts::TAU),
        _ => None,
    }
}

impl<'a> FnLowerer<'a> {
    /// `PI` or `TAU`, as a float literal of the expression's type.
    pub(crate) fn builtin_constant(b: Builtin) -> ExprKind {
        ExprKind::Float(constant_value(b).unwrap_or_else(|| unreachable!("{b:?} isn't a constant")))
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
