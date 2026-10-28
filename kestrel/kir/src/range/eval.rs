//! Expressions: each evaluates to a range (the spec's "Ranges of expressions" table) and, when it
//! has one, a linear form (see `bounds`), deciding the checks inside it on the way. Every operand
//! is analysed from the same state, so facts one operand makes (in an inlined call's `Seq`, a
//! value `if`'s condition, a check) never reach its siblings; only `Seq`s, `if`s and `and`/`or`
//! pass a state forward, in KIR's own order, and the expression they then evaluate first forgets
//! what it writes itself. The arms that need more than a line are functions of their own, so the
//! frame each level of nesting puts on the stack stays small.

use super::bounds::{Lin, Val};
use super::facts::ZERO;
use super::interval::{self as iv, Interval};
use super::sites;
use super::state::State;
use super::{terms, writes, Cx, MAX_ITEMS};
use crate::ir::*;

impl Cx<'_> {
    /// The range of `e`'s value in `st` (its type's range for a value that isn't an integer),
    /// deciding the checks in it. `sure` says `e` runs whenever its statement does, so a check in
    /// it that passes teaches the statement (see `learn`).
    pub(super) fn eval(&mut self, st: &State, e: &Expr, sure: bool) -> Interval {
        self.eval_v(st, e, sure).r
    }

    /// `eval`, with the value's linear form: a value whose range is one number is that number.
    pub(super) fn eval_v(&mut self, st: &State, e: &Expr, sure: bool) -> Val {
        if !self.enter(false) {
            return Val::range(Interval::of_type(e.ty));
        }
        let mut v = self.eval_kind(st, e, sure);
        self.leave();
        if !self.volatile.is_empty() {
            // Another read of a volatile node may be another value (see `within`).
            let ok = |l: &Lin| !self.volatile(l.node);
            v.lin = v.lin.filter(ok);
            v.between = v.between.filter(|(a, b)| ok(a) && ok(b));
            v.len = v.len.filter(ok);
        }
        if v.lin.is_none() && v.r.lo == v.r.hi && e.ty.is_int() {
            v.lin = Some(Lin { node: ZERO, c: v.r.lo });
        }
        v
    }

    fn eval_kind(&mut self, st: &State, e: &Expr, sure: bool) -> Val {
        let any = Val::range(Interval::of_type(e.ty));
        match &e.kind {
            ExprKind::Int(n) => Val::range(Interval::exact(sites::wrap_literal(e.ty, *n).into())),
            ExprKind::Bool(b) => Val::range(Interval::exact(i128::from(*b))),
            ExprKind::Load(p) => self.load(st, p, e.ty, sure),
            ExprKind::Len(p) => self.len(st, p, sure),
            // A list's capacity, or an array's length: nothing is known of it but its sign.
            ExprKind::Cap(p) => {
                self.place(st, p, sure);
                Val::range(Interval::new(0, i64::MAX.into()))
            }
            ExprKind::Move(p) | ExprKind::Tag(p) | ExprKind::FnEnv(p) | ExprKind::MakeIface { data: IfaceData::Borrowed(p), .. } => {
                self.place(st, p, sure);
                any
            }
            ExprKind::Map(_, p, xs) => {
                self.all(st, xs, sure);
                self.place(st, p, sure);
                any
            }
            ExprKind::Unary(UnOp::Neg, _) | ExprKind::Cast(_) => self.arith(st, e, sure, false),
            ExprKind::Binary(_, l, _) if l.ty.is_int() || l.ty == KTy::I32X4 => self.arith(st, e, sure, false),
            ExprKind::Proven(inner) => self.arith(st, inner, sure, true),
            ExprKind::VecLit(_) | ExprKind::Splat(_) | ExprKind::Shuffle(..) | ExprKind::LaneOf(..) => self.vector(st, e, sure),
            ExprKind::Unary(UnOp::BitNot, _) if e.ty == KTy::I32X4 => self.vector_arith(st, e, sure, false),
            ExprKind::Unary(UnOp::BitNot, x) if e.ty.is_int() => Val::range(iv::bit_not(self.eval(st, x, sure), e.ty)),
            ExprKind::Math(MathOp::Min | MathOp::Max, _) if e.ty == KTy::I32X4 => self.vector_math(st, e, sure),
            ExprKind::Unary(_, x)
            | ExprKind::Some(x)
            | ExprKind::EnvDup(x)
            | ExprKind::BlockDup { src: x, .. }
            | ExprKind::FieldOf(x, _)
            | ExprKind::MakeIface { data: IfaceData::Boxed(x), .. } => {
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
            ExprKind::RandInt(a, b) => self.rand_int(st, (a, b), e.ty, sure),
            ExprKind::Seq(stmts, v) => self.seq(st, stmts, v),
            ExprKind::If { cond, then_body, then_val, else_body, else_val } => {
                self.value_if(st, cond, (then_body, then_val.as_deref()), (else_body, else_val.as_deref()), e.ty)
            }
            ExprKind::Format(parts) => {
                self.print_args(st, parts, sure);
                any
            }
            ExprKind::ArrayNew(_) | ExprKind::ArrayLit(_) | ExprKind::ListNew => self.new_list(st, e, sure),
            ExprKind::StructLit(_, xs) | ExprKind::VariantLit(_, _, xs) | ExprKind::Math(_, xs) | ExprKind::Str(_, xs) => {
                self.all(st, xs, sure);
                any
            }
            ExprKind::MakeFn { env: EnvKind::Heap(env) | EnvKind::Stack(env), .. } => {
                self.all(st, &env.values, sure);
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
            | ExprKind::MapNew
            | ExprKind::NewArena
            | ExprKind::StaticArena
            | ExprKind::NoneLit
            | ExprKind::FnRef(_)
            | ExprKind::MakeFn { env: EnvKind::None, .. }
            | ExprKind::RandFloat { .. } => any,
        }
    }

    /// A load: a tracked local's range (and the local itself, when it is a term), a lane of a
    /// tracked `ivec4` local (`vector.rs`), or a list or array copied from a stable place (its
    /// length is that place's).
    #[inline(never)]
    fn load(&mut self, st: &State, p: &Place, ty: KTy, sure: bool) -> Val {
        self.place(st, p, sure);
        if let Some(v) = self.lane_load(st, p) {
            return v;
        }
        if p.path.is_empty() {
            if let Some(r) = st.get(&self.t, p.root) {
                return Val::exact(r, self.t.node(p.root).map(|node| Lin { node, c: 0 }));
            }
        }
        let len = terms::list_key(self.m, self.f, p, p.path.len()).and_then(|k| self.t.len_node(&k)).map(|node| Lin { node, c: 0 });
        Val { len, ..Val::range(Interval::of_type(ty)) }
    }

    /// `xs.len`: a list's or array's length is 0 to `MAX_ITEMS`, and a term when its place is
    /// stable; a string's is 0 or more.
    #[inline(never)]
    fn len(&mut self, st: &State, p: &Place, sure: bool) -> Val {
        self.place(st, p, sure);
        match terms::place_ty(self.m, self.f, p, p.path.len()) {
            KTy::List(_) | KTy::Array(_) => match terms::key_of(self.f, p, p.path.len()).and_then(|k| self.t.len_node(&k)) {
                Some(node) => Val::exact(st.node_range(&self.t, node), Some(Lin { node, c: 0 })),
                None => Val::range(Interval::new(0, MAX_ITEMS)),
            },
            _ => Val::range(Interval::new(0, i64::MAX.into())),
        }
    }

    /// `rand_int(a, b)`: between its two bounds, when neither wraps to the result's type.
    #[inline(never)]
    fn rand_int(&mut self, st: &State, (a, b): (&Expr, &Expr), ty: KTy, sure: bool) -> Val {
        let (a, b) = (self.eval_v(st, a, sure), self.eval_v(st, b, sure));
        let between = match (a.lin, b.lin) {
            (Some(x), Some(y)) if a.r.fits(ty) && b.r.fits(ty) => Some((x, y)),
            _ => None,
        };
        Val { between, ..Val::range(iv::rand_int(a.r, b.r, ty)) }
    }

    /// A `Seq`: its statements, then its value, which forgets what it writes first (it is an
    /// operand of its own). Its linear form is moved onto terms the statements didn't write.
    #[inline(never)]
    fn seq(&mut self, st: &State, stmts: &[Stmt], v: &Expr) -> Val {
        let mut after = self.stmts(st.clone(), stmts);
        if after.is_dead() {
            self.unreachable_expr(v);
            return Val::range(Interval::of_type(v.ty));
        }
        let w = writes::of_expr(v);
        self.forget(&mut after, &w);
        let val = self.within(&w, |cx| cx.eval_v(&after, v, false));
        self.rebase(&after, stmts, val)
    }

    /// A value `if`: the arms' ranges joined, each arm under its condition.
    #[inline(never)]
    fn value_if(&mut self, st: &State, cond: &Expr, then: (&[Stmt], Option<&Expr>), other: (&[Stmt], Option<&Expr>), ty: KTy) -> Val {
        let (t, f) = self.condition(st.clone(), cond);
        let a = self.arm(t, then.0, then.1);
        let b = self.arm(f, other.0, other.1);
        Val::range(match (a, b) {
            (Some(a), Some(b)) => a.join(b),
            (Some(r), None) | (None, Some(r)) => r,
            (None, None) => Interval::of_type(ty),
        })
    }

    /// A new list or array, with its length: `n` for `array[T](n)` (the runtime refuses a bad
    /// one), the number of items of a literal, 0 for `list[T]()`.
    #[inline(never)]
    fn new_list(&mut self, st: &State, e: &Expr, sure: bool) -> Val {
        let len = match &e.kind {
            ExprKind::ArrayNew(n) => self.eval_v(st, n, sure).lin,
            ExprKind::ArrayLit(xs) => {
                self.all(st, xs, sure);
                Some(Lin { node: ZERO, c: xs.len() as i128 })
            }
            _ => Some(Lin { node: ZERO, c: 0 }),
        };
        Val { len, ..Val::range(Interval::of_type(e.ty)) }
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

    /// One arm of a value `if`: its statements, then its value (`None` when it never gives one),
    /// which forgets what it writes first.
    fn arm(&mut self, st: State, body: &[Stmt], val: Option<&Expr>) -> Option<Interval> {
        let mut after = self.stmts(st, body);
        let val = val?;
        if after.is_dead() {
            self.unreachable_expr(val);
            return None;
        }
        let w = writes::of_expr(val);
        self.forget(&mut after, &w);
        Some(self.within(&w, |cx| cx.eval(&after, val, false)))
    }

    /// An expression no state reaches: in a deciding pass its checks are listed as kept.
    pub(super) fn unreachable_expr(&mut self, e: &Expr) {
        if self.deciding && !self.gave_up {
            self.keep += 1;
            let top = self.top.clone();
            self.eval(&top, e, false);
            self.keep -= 1;
        }
    }
}
