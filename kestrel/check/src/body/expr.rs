use kestrel_syntax::ast::{BinaryOp, Expr, ExprKind, UnaryOp};
use kestrel_syntax::Diagnostic;

use super::{BodyChecker, Expect};
use crate::tables::Res;
use crate::ty::Ty;

fn is_value_ty(t: Ty) -> bool {
    !matches!(t, Ty::Unit | Ty::Never | Ty::Error)
}

impl<'a> BodyChecker<'a> {
    /// Types `e`, using `want` only as a hint for literals and branches.
    /// Records the result in `Tables::expr_ty`.
    pub(crate) fn check_expr(&mut self, e: &Expr, want: Option<Ty>) -> Ty {
        let ty = self.expr_inner(e, want);
        self.t.expr_ty[e.id.0 as usize] = ty;
        ty
    }

    fn expr_inner(&mut self, e: &Expr, want: Option<Ty>) -> Ty {
        match &e.kind {
            ExprKind::Int(n) => self.int_literal(e, *n, want),
            ExprKind::Float(_) => {
                if want == Some(Ty::F64) {
                    Ty::F64
                } else {
                    Ty::Float
                }
            }
            ExprKind::Bool(_) => Ty::Bool,
            ExprKind::Str(_) => {
                self.error(e.span, "string values aren't supported yet (only as `print` arguments)");
                Ty::Error
            }
            ExprKind::Ident(name) => self.ident(e, name),
            ExprKind::Unary { op, expr } => self.unary(*op, expr, want),
            ExprKind::Binary { op, lhs, rhs } => self.binary(e, *op, lhs, rhs, want),
            ExprKind::Call { .. } => self.check_call(e, want),
            ExprKind::If { .. } => self.check_if(e, Expect::Value(want)),
            ExprKind::Field { .. } | ExprKind::MethodCall { .. } => {
                self.error(e.span, "structs aren't supported yet");
                Ty::Error
            }
        }
    }

    fn int_literal(&mut self, e: &Expr, n: u64, want: Option<Ty>) -> Ty {
        let ty = match want {
            Some(t) if t.is_numeric() => t,
            _ => Ty::Int,
        };
        let max = match ty {
            Ty::U8 => u8::MAX as u64,
            Ty::Int => i32::MAX as u64,
            Ty::I64 => i64::MAX as u64,
            _ => return ty,
        };
        if n > max {
            let mut d = Diagnostic::error(e.span, format!("`{n}` doesn't fit in {}", ty.name()));
            if ty == Ty::Int {
                d = d.with_help("annotate it as i64: `let x: i64 = ...`");
            }
            self.diags.push(d);
            return Ty::Error;
        }
        ty
    }

    fn ident(&mut self, e: &Expr, name: &str) -> Ty {
        if let Some(local) = self.lookup_local(name) {
            self.t.res.insert(e.id, Res::Local(local));
            return self.t.locals[local.0 as usize].ty;
        }
        if let Some(&c) = self.t.const_by_name.get(name) {
            self.t.res.insert(e.id, Res::Const(c));
            return self.t.consts[c.0 as usize].ty;
        }
        if self.t.fn_by_name.contains_key(name) {
            self.error(e.span, format!("`{name}` is a function; call it with `{name}(...)`"));
        } else {
            self.error(e.span, format!("unknown name `{name}`"));
        }
        Ty::Error
    }

    fn unary(&mut self, op: UnaryOp, inner: &Expr, want: Option<Ty>) -> Ty {
        match op {
            UnaryOp::Not => {
                self.check_value(inner, Some(Ty::Bool));
                Ty::Bool
            }
            UnaryOp::Neg => {
                let ty = self.check_expr(inner, want.filter(|t| t.is_numeric()));
                if ty == Ty::U8 {
                    self.error(inner.span, "can't negate a u8");
                    return Ty::Error;
                }
                if !ty.is_numeric() && ty != Ty::Error {
                    self.error(inner.span, format!("`-` needs a number, found {}", ty.name()));
                    return Ty::Error;
                }
                ty
            }
        }
    }

    fn binary(&mut self, e: &Expr, op: BinaryOp, lhs: &Expr, rhs: &Expr, want: Option<Ty>) -> Ty {
        match op {
            BinaryOp::And | BinaryOp::Or => {
                self.check_value(lhs, Some(Ty::Bool));
                self.check_value(rhs, Some(Ty::Bool));
                Ty::Bool
            }
            _ if op.is_comparison() => {
                let lt = self.check_expr(lhs, None);
                let rt = self.check_expr(rhs, Some(lt).filter(|t| t.is_numeric()));
                if lt == Ty::Bool && rt == Ty::Bool && matches!(op, BinaryOp::Eq | BinaryOp::Ne) {
                    return Ty::Bool;
                }
                self.operands(e, op, lhs, lt, rhs, rt);
                Ty::Bool
            }
            _ => {
                let hint = want.filter(|t| t.is_numeric());
                let lt = self.check_expr(lhs, hint);
                let rt = self.check_expr(rhs, Some(lt).filter(|t| t.is_numeric()).or(hint));
                self.operands(e, op, lhs, lt, rhs, rt)
            }
        }
    }

    /// Both operands must be numbers that meet at a common type.
    fn operands(&mut self, e: &Expr, op: BinaryOp, lhs: &Expr, lt: Ty, rhs: &Expr, rt: Ty) -> Ty {
        if lt == Ty::Error || rt == Ty::Error {
            return Ty::Error;
        }
        let sym = op.symbol();
        if !lt.is_numeric() || !rt.is_numeric() {
            self.error(e.span, format!("`{sym}` needs numbers, found {} and {}", lt.name(), rt.name()));
            return Ty::Error;
        }
        match Ty::common(lt, rt) {
            Some(c) => {
                self.coerce(lhs, lt, c);
                self.coerce(rhs, rt, c);
                c
            }
            None => {
                let msg = format!("`{sym}` can't mix {} and {}", lt.name(), rt.name());
                self.error_help(e.span, msg, "convert one side explicitly, e.g. `int(x)` or `u8(x)`");
                Ty::Error
            }
        }
    }

    pub(crate) fn check_if(&mut self, e: &Expr, expect: Expect) -> Ty {
        let ExprKind::If { cond, then_block, else_block } = &e.kind else { unreachable!("check_if on non-if") };
        self.check_value(cond, Some(Ty::Bool));
        match expect {
            Expect::Discard => {
                let t1 = self.check_block(then_block, Expect::Discard);
                let t2 = match else_block {
                    Some(b) => self.check_block(b, Expect::Discard),
                    None => Ty::Unit,
                };
                if t1 == Ty::Never && t2 == Ty::Never {
                    Ty::Never
                } else {
                    Ty::Unit
                }
            }
            Expect::Value(want) => {
                let Some(else_block) = else_block else {
                    // Checked as a value so its last line isn't also flagged "no effect".
                    self.check_block(then_block, Expect::Value(want));
                    self.error(e.span, "this `if` needs an `else` to produce a value");
                    return Ty::Error;
                };
                let t1 = self.check_block(then_block, Expect::Value(want));
                let hint = want.or(if is_value_ty(t1) { Some(t1) } else { None });
                let t2 = self.check_block(else_block, Expect::Value(hint));
                // A branch that finishes without a value would bypass the value checks.
                let mut missing = false;
                for (ty, block) in [(t1, then_block), (t2, else_block)] {
                    if ty == Ty::Unit {
                        self.error(block.span, "this branch has no value");
                        missing = true;
                    }
                }
                if missing {
                    return Ty::Error;
                }
                if t1 == Ty::Never && t2 == Ty::Never {
                    Ty::Never
                } else if let Some(w) = want {
                    w
                } else if t1 == Ty::Never {
                    t2
                } else {
                    t1
                }
            }
        }
    }
}
