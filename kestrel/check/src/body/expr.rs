use kestrel_syntax::ast::{BinaryOp, Expr, ExprKind, InterpPart, UnaryOp};
use kestrel_syntax::Diagnostic;

use super::{BodyChecker, Expect};
use crate::tables::Res;
use crate::ty::Ty;

pub(super) fn is_value_ty(t: Ty) -> bool {
    !matches!(t, Ty::Unit | Ty::Never | Ty::Error)
}

impl<'a> BodyChecker<'a> {
    /// Types `e`, using `want` only as a hint for literals and branches.
    /// Records the result in `Tables::expr_ty`.
    pub(crate) fn check_expr(&mut self, e: &Expr, want: Option<Ty>) -> Ty {
        let ty = self.expr_inner(e, want);
        self.t.expr_ty[e.id.0 as usize] = ty;
        if matches!(e.kind, ExprKind::Ident(_) | ExprKind::Field { .. } | ExprKind::Index { .. }) {
            self.note_par_read(e);
        }
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
            ExprKind::None => match want {
                Some(t @ Ty::Optional(_)) => t,
                _ => {
                    self.error_help(e.span, "can't tell what kind of `none` this is", "give it a type: `let x: int? = none`");
                    Ty::Error
                }
            },
            ExprKind::Str(_) => Ty::Str,
            ExprKind::Interp(parts) => {
                // Building a new string needs memory to put it in; that comes with arenas.
                self.error(e.span, "string interpolation only works directly inside `print(...)` for now");
                for part in parts {
                    if let InterpPart::Expr(x) = part {
                        self.check_expr(x, None);
                    }
                }
                Ty::Error
            }
            ExprKind::Ident(name) => self.ident(e, name),
            ExprKind::Unary { op, expr } => self.unary(*op, expr, want),
            ExprKind::Binary { op, lhs, rhs } => self.binary(e, *op, lhs, rhs, want),
            ExprKind::Call { .. } => self.check_call(e, want),
            ExprKind::If { .. } => self.check_if(e, Expect::Value(want)),
            ExprKind::Field { .. } => self.check_field(e),
            ExprKind::MethodCall { .. } => self.check_method_call(e),
            ExprKind::Match { .. } => self.check_match(e, Expect::Value(want)),
            ExprKind::Index { .. } => self.check_index(e),
            ExprKind::ArrayLit(items) => self.check_array_lit(e, items, want),
            ExprKind::MapLit(entries) => self.check_map_lit(e, entries, want),
            ExprKind::TypeCall { .. } => self.check_type_call(e),
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
            let mut d = Diagnostic::error(e.span, format!("`{n}` doesn't fit in {}", self.tn(ty)));
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
            if self.moved.contains(&local) {
                self.error(e.span, format!("`{name}` was moved earlier, so it can't be used here"));
            }
            let ty = self.t.locals[local.0 as usize].ty;
            if let Ty::Variant(..) = ty {
                let msg = format!("`{name}` stands for the fields of `{}`; use them as `{name}.field`", self.tn(ty));
                self.error(e.span, msg);
                return Ty::Error;
            }
            return ty;
        }
        if let Some(&c) = self.t.const_by_name.get(name) {
            self.t.res.insert(e.id, Res::Const(c));
            return self.t.consts[c.0 as usize].ty;
        }
        if self.t.fn_by_name.contains_key(name) {
            self.error(e.span, format!("`{name}` is a function; call it with `{name}(...)`"));
        } else if self.t.struct_by_name.contains_key(name) {
            self.error(e.span, format!("`{name}` is a struct; build one with `{name}(...)`"));
        } else if let Some(&id) = self.t.enum_by_name.get(name) {
            let example = match self.t.enm(id).variants.first() {
                Some(v) => format!(", like `{name}.{}`", v.name),
                None => String::new(),
            };
            self.error(e.span, format!("`{name}` is an enum; use one of its variants{example}"));
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
            UnaryOp::BitNot => {
                let ty = self.check_expr(inner, want.filter(|t| t.is_int()));
                if !ty.is_int() && ty != Ty::Error {
                    let msg = format!("`~` needs an integer, found {}", self.tn(ty));
                    self.error(inner.span, msg);
                    return Ty::Error;
                }
                ty
            }
            UnaryOp::Neg => {
                let ty = self.check_expr(inner, want.filter(|t| t.is_numeric()));
                if self.t.vec_dims(ty).is_some() {
                    return ty;
                }
                if ty == Ty::U8 {
                    self.error(inner.span, "can't negate a u8");
                    return Ty::Error;
                }
                if !ty.is_numeric() && ty != Ty::Error {
                    let msg = format!("`-` needs a number, found {}", self.tn(ty));
                    self.error(inner.span, msg);
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
            // A shift's result has the left side's type; the amount is any integer.
            BinaryOp::Shl | BinaryOp::Shr => {
                let lt = self.check_expr(lhs, want.filter(|t| t.is_int()));
                let rt = self.check_expr(rhs, Some(lt).filter(|t| t.is_int()));
                self.bit_operands(op, lhs, lt, rhs, rt);
                if lt.is_int() { lt } else { Ty::Error }
            }
            _ if op.is_bitwise() => {
                let lt = self.check_expr(lhs, want.filter(|t| t.is_int()));
                let rt = self.check_expr(rhs, Some(lt).filter(|t| t.is_int()));
                if !self.bit_operands(op, lhs, lt, rhs, rt) {
                    return Ty::Error;
                }
                self.operands(e, op, lhs, lt, rhs, rt)
            }
            _ if op.is_comparison() => {
                let lt = self.check_expr(lhs, None);
                let rt = self.check_expr(rhs, Some(lt).filter(|t| t.is_numeric()));
                if matches!(op, BinaryOp::Eq | BinaryOp::Ne) && [lt, rt].iter().any(|t| matches!(t, Ty::Bool | Ty::Str)) {
                    if lt != rt && lt != Ty::Error && rt != Ty::Error {
                        let msg = format!("`{}` needs two numbers, two bools or two strings, found {} and {}", op.symbol(), self.tn(lt), self.tn(rt));
                        self.error(e.span, msg);
                    }
                    return Ty::Bool;
                }
                self.operands(e, op, lhs, lt, rhs, rt);
                Ty::Bool
            }
            _ => {
                let hint = want.filter(|t| t.is_numeric());
                let lt = self.check_expr(lhs, hint);
                let rt = self.check_expr(rhs, Some(lt).filter(|t| t.is_numeric()).or(hint));
                if self.t.vec_dims(lt).is_some() || self.t.vec_dims(rt).is_some() {
                    return self.vec_binary(e, op, lhs, lt, rhs, rt);
                }
                self.operands(e, op, lhs, lt, rhs, rt)
            }
        }
    }

    /// `+ - * /` on vectors: component-wise with a vector of the same size,
    /// or with a number (applied to every component).
    fn vec_binary(&mut self, e: &Expr, op: BinaryOp, lhs: &Expr, lt: Ty, rhs: &Expr, rt: Ty) -> Ty {
        if lt == Ty::Error || rt == Ty::Error {
            return Ty::Error;
        }
        let sym = op.symbol();
        if !matches!(op, BinaryOp::Add | BinaryOp::Sub | BinaryOp::Mul | BinaryOp::Div) {
            self.error(e.span, format!("`{sym}` doesn't work on vectors"));
            return Ty::Error;
        }
        let (vec, scalar, scalar_ty) = match (self.t.vec_dims(lt), self.t.vec_dims(rt)) {
            (Some(_), Some(_)) if lt == rt => return lt,
            (Some(_), Some(_)) => {
                let msg = format!("`{sym}` can't mix {} and {}", self.tn(lt), self.tn(rt));
                self.error(e.span, msg);
                return Ty::Error;
            }
            (Some(_), None) => (lt, rhs, rt),
            _ => (rt, lhs, lt),
        };
        if !scalar_ty.is_numeric() {
            let msg = format!("`{sym}` needs a vector and a number, found {} and {}", self.tn(lt), self.tn(rt));
            self.error(e.span, msg);
            return Ty::Error;
        }
        self.coerce(scalar, scalar_ty, Ty::Float);
        vec
    }

    /// Bit operators take integers. Returns whether both sides are.
    fn bit_operands(&mut self, op: BinaryOp, lhs: &Expr, lt: Ty, rhs: &Expr, rt: Ty) -> bool {
        for (e, t) in [(lhs, lt), (rhs, rt)] {
            if !t.is_int() && t != Ty::Error {
                let msg = format!("`{}` needs integers, found {}", op.symbol(), self.tn(t));
                self.error(e.span, msg);
                return false;
            }
        }
        lt.is_int() && rt.is_int()
    }

    /// Both operands must be numbers that meet at a common type.
    fn operands(&mut self, e: &Expr, op: BinaryOp, lhs: &Expr, lt: Ty, rhs: &Expr, rt: Ty) -> Ty {
        if lt == Ty::Error || rt == Ty::Error {
            return Ty::Error;
        }
        let sym = op.symbol();
        if !lt.is_numeric() || !rt.is_numeric() {
            let msg = format!("`{sym}` needs numbers, found {} and {}", self.tn(lt), self.tn(rt));
            self.error(e.span, msg);
            return Ty::Error;
        }
        match Ty::common(lt, rt) {
            Some(c) => {
                self.coerce(lhs, lt, c);
                self.coerce(rhs, rt, c);
                c
            }
            None => {
                let msg = format!("`{sym}` can't mix {} and {}", self.tn(lt), self.tn(rt));
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
