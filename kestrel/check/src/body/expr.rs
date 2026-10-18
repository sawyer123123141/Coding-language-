use kestrel_syntax::ast::{Expr, ExprKind, InterpPart};
use kestrel_syntax::{Diagnostic, Span};

use super::{BodyChecker, Expect};
use super::fnvalue::builtin_wrapper;
use crate::builtins::Builtin;
use crate::tables::{LocalId, Res};
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
            self.note_remove_read(e);
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
            // Built in the current arena, from parts that print the way `print` shows them.
            ExprKind::Interp(parts) => {
                for part in parts {
                    if let InterpPart::Expr(x) = part {
                        self.check_printable(x);
                    }
                }
                Ty::Str
            }
            ExprKind::Ident(name) => self.ident(e, name, want),
            ExprKind::Unary { op, expr } => self.unary(*op, expr, want),
            ExprKind::Binary { op, lhs, rhs } => self.binary(e, *op, lhs, rhs, want),
            ExprKind::Call { .. } => self.check_call(e, want),
            ExprKind::CallValue { .. } => self.check_call_value(e),
            ExprKind::Closure(lit) => self.check_closure(e, lit, want),
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

    /// A use of `local`: it must not have been moved or removed on the way here, and a view
    /// into a loop item can't be used once the item is removed.
    pub(crate) fn check_local_use(&mut self, local: LocalId, name: &str, span: Span) {
        self.refuse_capture(local, name, span);
        match self.moved.get(&local).copied() {
            Some(at) if at.removed && at.every => self.error(span, format!("`{name}` was removed earlier, so it can't be used here")),
            Some(at) if at.removed => self.error_help(
                span,
                format!("`{name}` may have been removed earlier, so it can't be used here"),
                "it's removed on some paths to this line",
            ),
            Some(at) if at.every => self.error(span, format!("`{name}` was moved earlier, so it can't be used here")),
            Some(_) => self.error_help(
                span,
                format!("`{name}` may have been moved earlier, so it can't be used here"),
                "it's moved on some paths to this line; give it a new value first",
            ),
            // A view into a loop item reads freed memory once the item is removed.
            None => self.check_removed_view(local, span),
        }
    }

    fn ident(&mut self, e: &Expr, name: &str, want: Option<Ty>) -> Ty {
        if let Some(local) = self.lookup_local(name) {
            self.t.res.insert(e.id, Res::Local(local));
            self.check_local_use(local, name, e.span);
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
        if let Some(ty) = self.builtin_constant(e, name, want) {
            return ty;
        }
        if let Some(&fid) = self.t.fn_by_name.get(name) {
            return self.fn_name_value(e, fid);
        }
        if name == "print" || Builtin::global(name).is_some() {
            let msg = format!("`{name}` is built in; wrap it: `{}`", builtin_wrapper(name));
            self.error(e.span, msg);
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

    pub(crate) fn check_if(&mut self, e: &Expr, expect: Expect) -> Ty {
        let ExprKind::If { cond, then_block, else_block } = &e.kind else { unreachable!("check_if on non-if") };
        self.check_value(cond, Some(Ty::Bool));
        let before = self.moved.clone();
        match expect {
            Expect::Discard => {
                let t1 = self.check_block(then_block, Expect::Discard);
                let after_then = std::mem::replace(&mut self.moved, before.clone());
                let t2 = match else_block {
                    Some(b) => self.check_block(b, Expect::Discard),
                    None => Ty::Unit,
                };
                let after_else = std::mem::take(&mut self.moved);
                self.join(before, vec![(t1, after_then), (t2, after_else)]);
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
                let after_then = std::mem::replace(&mut self.moved, before.clone());
                let hint = want.or(if is_value_ty(t1) { Some(t1) } else { None });
                let t2 = self.check_block(else_block, Expect::Value(hint));
                let after_else = std::mem::take(&mut self.moved);
                self.join(before, vec![(t1, after_then), (t2, after_else)]);
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

    /// An expression whose value nobody uses.
    pub(crate) fn check_discarded(&mut self, e: &Expr) -> Ty {
        let ty = match &e.kind {
            ExprKind::If { .. } => self.check_if(e, Expect::Discard),
            ExprKind::Match { .. } => self.check_match(e, Expect::Discard),
            ExprKind::Call { .. } | ExprKind::MethodCall { .. } | ExprKind::CallValue { .. } => self.check_discarded_call(e),
            _ => {
                let ty = self.check_expr(e, None);
                if ty != Ty::Error {
                    self.error(e.span, "this line has no effect; did you mean to continue the line above?");
                }
                ty
            }
        };
        if ty == Ty::Never {
            Ty::Never
        } else {
            Ty::Unit
        }
    }
}
