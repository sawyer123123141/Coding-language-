//! Expressions: each evaluates to a range (the spec's "Ranges of expressions" table), deciding the
//! checks inside it on the way. Every operand is analysed from the same state, so facts one operand
//! makes (in an inlined call's `Seq`, a value `if`'s condition, a check) never reach its siblings;
//! only `Seq`s, `if`s and `and`/`or` pass a state forward, in KIR's own order.

use super::interval::{self as iv, Interval};
use super::sites::{self, CheckKind};
use super::state::State;
use super::{expr_key, index_key, Cx, MAX_ITEMS};
use crate::ir::*;

impl Cx<'_> {
    /// The range of `e`'s value in `st` (its type's range for a value that isn't an integer),
    /// deciding the checks in it. `sure` says `e` runs whenever its statement does, so a check in
    /// it that passes teaches the statement (see `learn`).
    pub(super) fn eval(&mut self, st: &State, e: &Expr, sure: bool) -> Interval {
        if !self.enter(false) {
            return Interval::of_type(e.ty);
        }
        let r = self.eval_kind(st, e, sure);
        self.leave();
        r
    }

    fn eval_kind(&mut self, st: &State, e: &Expr, sure: bool) -> Interval {
        let any = Interval::of_type(e.ty);
        match &e.kind {
            ExprKind::Int(n) => Interval::exact(sites::wrap_literal(e.ty, *n).into()),
            ExprKind::Bool(b) => Interval::exact(i128::from(*b)),
            ExprKind::Load(p) => {
                self.place(st, p, sure);
                match p.path.is_empty() {
                    true => st.get(&self.t, p.root).unwrap_or(any),
                    false => any,
                }
            }
            ExprKind::Len(p) => {
                self.place(st, p, sure);
                match self.place_ty(p) {
                    KTy::List(_) | KTy::Array(_) => Interval::new(0, MAX_ITEMS),
                    _ => Interval::new(0, i64::MAX.into()),
                }
            }
            ExprKind::Move(p) | ExprKind::Tag(p) | ExprKind::FnEnv(p) => {
                self.place(st, p, sure);
                any
            }
            ExprKind::Map(_, p, xs) => {
                self.all(st, xs, sure);
                self.place(st, p, sure);
                any
            }
            ExprKind::Unary(UnOp::Neg, _) | ExprKind::Cast(_) => self.arith(st, e, sure, false),
            ExprKind::Binary(_, l, _) if l.ty.is_int() => self.arith(st, e, sure, false),
            ExprKind::Proven(inner) => self.arith(st, inner, sure, true),
            ExprKind::Unary(UnOp::BitNot, x) if e.ty.is_int() => iv::bit_not(self.eval(st, x, sure), e.ty),
            ExprKind::Unary(_, x) => {
                self.eval(st, x, sure);
                any
            }
            ExprKind::Binary(_, l, r) => {
                self.eval(st, l, sure);
                self.eval(st, r, sure);
                any
            }
            ExprKind::And(..) | ExprKind::Or(..) => {
                let _ = self.cond_parts(st.clone(), e, sure);
                any
            }
            ExprKind::RandInt(a, b) => {
                let (a, b) = (self.eval(st, a, sure), self.eval(st, b, sure));
                iv::rand_int(a, b, e.ty)
            }
            ExprKind::Seq(stmts, v) => {
                let after = self.stmts(st.clone(), stmts);
                if after.is_dead() {
                    self.unreachable_expr(v);
                    return any;
                }
                self.eval(&after, v, false)
            }
            ExprKind::If { cond, then_body, then_val, else_body, else_val } => {
                let (t, f) = self.condition(st.clone(), cond);
                let a = self.arm(t, then_body, then_val.as_deref());
                let b = self.arm(f, else_body, else_val.as_deref());
                match (a, b) {
                    (Some(a), Some(b)) => a.join(b),
                    (Some(r), None) | (None, Some(r)) => r,
                    (None, None) => any,
                }
            }
            ExprKind::Format(parts) => {
                self.print_args(st, parts, sure);
                any
            }
            ExprKind::ArrayNew(x)
            | ExprKind::Some(x)
            | ExprKind::EnvDup(x)
            | ExprKind::BlockDup { src: x, .. }
            | ExprKind::FieldOf(x, _)
            | ExprKind::MakeIface { data: IfaceData::Boxed(x), .. } => {
                self.eval(st, x, sure);
                any
            }
            ExprKind::ArrayLit(xs) | ExprKind::StructLit(_, xs) | ExprKind::VariantLit(_, _, xs) | ExprKind::Math(_, xs) | ExprKind::Str(_, xs) => {
                self.all(st, xs, sure);
                any
            }
            ExprKind::MakeFn { env: EnvKind::Heap(env) | EnvKind::Stack(env), .. } => {
                self.all(st, &env.values, sure);
                any
            }
            ExprKind::MakeIface { data: IfaceData::Borrowed(p), .. } => {
                self.place(st, p, sure);
                any
            }
            ExprKind::Call(_, args) => {
                self.args(st, args, sure);
                any
            }
            ExprKind::CallIndirect { callee: p, args, .. } | ExprKind::CallVirtual { recv: p, args, .. } => {
                self.place(st, p, sure);
                self.args(st, args, sure);
                any
            }
            ExprKind::Float(_)
            | ExprKind::StrLit(_)
            | ExprKind::ListNew
            | ExprKind::MapNew
            | ExprKind::NewArena
            | ExprKind::StaticArena
            | ExprKind::NoneLit
            | ExprKind::FnRef(_)
            | ExprKind::MakeFn { env: EnvKind::None, .. }
            | ExprKind::RandFloat => any,
        }
    }

    /// An integer `-x`, binary operation or conversion: its range, and its check decided (unless
    /// `already` proven: a module analysed before).
    fn arith(&mut self, st: &State, e: &Expr, sure: bool, already: bool) -> Interval {
        let checked = self.arith.checked();
        let any = Interval::of_type(e.ty);
        match &e.kind {
            ExprKind::Unary(UnOp::Neg, x) => {
                let a = self.eval(st, x, sure);
                if !e.ty.is_int() {
                    return any;
                }
                let exact = iv::neg(a);
                self.site(e, CheckKind::Overflow, exact.fits(e.ty), already, &[(x, a)]);
                iv::settle(exact, e.ty, checked)
            }
            ExprKind::Binary(op, l, r) => {
                let (a, b) = (self.eval(st, l, sure), self.eval(st, r, sure));
                let ty = l.ty;
                if !ty.is_int() || op.is_comparison() {
                    return any;
                }
                let operands = [(&**l, a), (&**r, b)];
                match op {
                    BinOp::Add | BinOp::Sub | BinOp::Mul => {
                        let exact = match op {
                            BinOp::Add => iv::add(a, b),
                            BinOp::Sub => iv::sub(a, b),
                            _ => iv::mul(a, b),
                        };
                        self.site(e, CheckKind::Overflow, exact.fits(ty), already, &operands);
                        iv::settle(exact, ty, checked)
                    }
                    BinOp::Div | BinOp::Rem => {
                        let safe = iv::division_safe(a, b, ty);
                        if sites::divides_checked(ty, r) {
                            self.site(e, CheckKind::Division, safe, already, &operands);
                            if sure && !already {
                                // The zero check passed (in both builds): the divisor isn't 0.
                                self.learn(r, nonzero(b));
                            }
                        }
                        let exact = if *op == BinOp::Div { iv::div(a, b, ty) } else { iv::rem(a, b, ty) };
                        iv::settle(exact, ty, checked)
                    }
                    BinOp::Shl | BinOp::Shr => {
                        let safe = iv::shift_safe(b, ty);
                        if checked {
                            self.site(e, CheckKind::Shift, safe, already, &operands);
                            if sure && !already {
                                self.learn(r, Some(Interval::new(0, i128::from(iv::width(ty)) - 1)));
                            }
                        }
                        let n = iv::shift_amount(b, ty, checked);
                        if *op == BinOp::Shl {
                            iv::shl(a, n, ty)
                        } else {
                            iv::shr(a, n)
                        }
                    }
                    BinOp::BitAnd => iv::bit_and(a, b, ty),
                    BinOp::BitOr | BinOp::BitXor => iv::bit_or_xor(a, b, ty, *op == BinOp::BitXor),
                    _ => any,
                }
            }
            ExprKind::Cast(x) => {
                let a = self.eval(st, x, sure);
                let (from, to) = (x.ty, e.ty);
                if !to.is_int() {
                    return any;
                }
                if from.is_float() {
                    return any; // saturates: anything of the target type
                }
                if a.fits(to) {
                    // A widening, or a narrowing that can't fail.
                    if sites::is_narrowing(from, to) && checked {
                        self.site(e, CheckKind::Narrowing, true, already, &[(x, a)]);
                    }
                    return a;
                }
                if checked && sites::is_narrowing(from, to) {
                    self.site(e, CheckKind::Narrowing, false, already, &[(x, a)]);
                    if sure && !already {
                        self.learn(x, Some(any));
                    }
                }
                iv::settle(a, to, checked && sites::is_narrowing(from, to))
            }
            _ => self.eval_kind(st, e, sure),
        }
    }

    /// Decides the check of kind `kind` on `e` (`safe`: it can't fail), unless it was proven
    /// `already` or codegen doesn't emit it in this build.
    fn site(&mut self, e: &Expr, kind: CheckKind, safe: bool, already: bool, operands: &[(&Expr, Interval)]) {
        // Only a check codegen emits in this build is a site.
        if !already && sites::checks_of(e, self.arith).contains(&kind) {
            self.decide(expr_key(e), kind, e.line, safe, |cx| super::show::check_text(cx.m, cx.f, kind, e, operands));
        }
    }

    /// After a check on `e` passed, `e`'s value is in `range` (when there is one): a fact about
    /// the local `e` reads, if it reads one directly (or through a widening conversion).
    fn learn(&mut self, e: &Expr, range: Option<Interval>) {
        if let (Some(l), Some(r)) = (self.local_of(e), range) {
            self.learned.push((l, r));
        }
    }

    /// The tracked local whose value `e` is, if it is one: a load of it, or a widening conversion
    /// of one.
    pub(super) fn local_of(&self, e: &Expr) -> Option<LocalIdx> {
        match &e.kind {
            ExprKind::Load(p) if p.path.is_empty() && self.t.is_tracked(p.root) => Some(p.root),
            ExprKind::Cast(x) if x.ty.is_int() && e.ty.is_int() && !sites::is_narrowing(x.ty, e.ty) => self.local_of(x),
            _ => None,
        }
    }

    /// The index expressions of a place, and its bounds checks.
    pub(super) fn place(&mut self, st: &State, p: &Place, sure: bool) {
        for (i, proj) in p.path.iter().enumerate() {
            let Some(at) = proj.at() else { continue };
            let r = self.eval(st, at, sure);
            if sites::is_bounds_site(proj) {
                self.decide(index_key(proj), CheckKind::Bounds, at.line, false, |cx| super::show::bounds_text(cx.m, cx.f, p, i, Some(r)));
            }
        }
    }

    pub(super) fn all(&mut self, st: &State, xs: &[Expr], sure: bool) {
        for x in xs {
            self.eval(st, x, sure);
        }
    }

    pub(super) fn args(&mut self, st: &State, args: &[Arg], sure: bool) {
        for a in args {
            match a {
                Arg::Value(e) => {
                    self.eval(st, e, sure);
                }
                Arg::Ref(p) => self.place(st, p, sure),
            }
        }
    }

    pub(super) fn print_args(&mut self, st: &State, parts: &[PrintArg], sure: bool) {
        for p in parts {
            match p {
                PrintArg::Value(e) => {
                    self.eval(st, e, sure);
                }
                PrintArg::Interp(inner) => self.print_args(st, inner, sure),
                PrintArg::Str(_) => {}
            }
        }
    }

    /// One arm of a value `if`: its statements, then its value (`None` when it never gives one).
    fn arm(&mut self, st: State, body: &[Stmt], val: Option<&Expr>) -> Option<Interval> {
        let after = self.stmts(st, body);
        let val = val?;
        if after.is_dead() {
            self.unreachable_expr(val);
            return None;
        }
        Some(self.eval(&after, val, false))
    }

    /// An expression no state reaches: in a deciding pass its checks are listed as kept.
    pub(super) fn unreachable_expr(&mut self, e: &Expr) {
        if self.deciding && !self.gave_up {
            self.keep += 1;
            let top = self.t.top();
            self.eval(&top, e, false);
            self.keep -= 1;
        }
    }
}

/// What a passed zero check says about a divisor in `b`: it isn't 0, which narrows an end at 0.
fn nonzero(b: Interval) -> Option<Interval> {
    if b.lo == 0 && b.hi > 0 {
        Some(Interval::new(1, b.hi))
    } else if b.hi == 0 && b.lo < 0 {
        Some(Interval::new(b.lo, -1))
    } else {
        None
    }
}
