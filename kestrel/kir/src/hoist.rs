//! Moves pure math whose operands a loop never changes out of the loop. Cranelift never moves a
//! call, or anything with control flow, out of a loop, so `pow(KEEP, dt)` in a particle loop (a
//! runtime call) or `sin(x)` (inline code with a slow branch) would run once per item.
//!
//! What moves is the largest subexpression that is invariant (see `invariant`) and holds a
//! transcendental `Math` or a float `%` (see `costly`). It goes to a new `Value` local, bound just
//! before the loop. Such an expression cannot fail (a float `/` or `%` by zero gives inf or NaN,
//! never a trap) and changes nothing, so it may leave a loop that runs zero times, or an `if`
//! inside the loop; it is deterministic, so it gives the same bits before the loop. It has no
//! check and no event, so range analysis and contracts see the same program, and the report
//! writes a load of the new local as the expression it replaced (`Module::hoisted`).
//!
//! The inliner binds each argument of an inlined call to a temporary inside the loop
//! (`let dt2 = dt`), so `pow(KEEP, dt2)` reads a local the loop writes. Such temporaries are seen
//! through, one level deep (see `temps`): the hoisted copy reads the temporary's initializer.

use std::collections::{HashMap, HashSet};

use crate::ir::*;
use crate::visit::{walk, walk_expr, Node};
use crate::visit_mut::{walk_expr as walk_expr_mut, walk_place, walk_stmt, VisitMut};

/// Moves each loop-invariant expression that holds a transcendental `Math` or a float `%` into a
/// new `Value` local bound just before the loop (spec decision 5), and records in
/// `Module::hoisted` the expression each new local replaced. Runs per function, innermost loop
/// first; `par for` bodies are functions of their own.
pub fn hoist_math(m: &mut Module) {
    let Module { funcs, hoisted, .. } = m;
    for (i, f) in funcs.iter_mut().enumerate().filter(|(_, f)| f.native.is_none()) {
        let mut body = std::mem::take(&mut f.body);
        let mut loops = Loops { locals: &mut f.locals, replaced: Vec::new() };
        loops.visit_stmts(&mut body);
        f.body = body;
        hoisted.extend(loops.replaced.into_iter().map(|(l, e)| ((FuncIdx(i as u32), l), e)));
    }
}

/// Finds the loops of a function, innermost first, and hoists out of each. A `Let` hoisted out of
/// an inner loop lands in the outer loop's body, where the outer loop's pass may move its
/// initializer on (an inliner temporary of the inner loop is seen through only by the inner pass),
/// so an expression leaves every loop it is invariant in.
struct Loops<'a> {
    locals: &'a mut Vec<Local>,
    /// Each new local, with the expression it replaced.
    replaced: Vec<(LocalIdx, Expr)>,
}

impl VisitMut for Loops<'_> {
    fn visit_stmts(&mut self, list: &mut Vec<Stmt>) {
        let mut i = 0;
        while i < list.len() {
            // Into the statement first: a loop nested inside gets its own pass before this one.
            self.visit_stmt(&mut list[i]);
            if matches!(list[i], Stmt::While { .. } | Stmt::ForRange { .. }) {
                let lets = hoist_loop(&mut list[i], self.locals, &mut self.replaced);
                let n = lets.len();
                list.splice(i..i, lets);
                i += n;
            }
            i += 1;
        }
    }
}

/// Replaces the hoistable expressions of the loop `s` with loads of new locals, and returns the
/// `Let`s that bind them, in the order found, to go just before the loop. Each new local and the
/// expression it replaced go into `replaced`.
fn hoist_loop(s: &mut Stmt, locals: &mut Vec<Local>, replaced: &mut Vec<(LocalIdx, Expr)>) -> Vec<Stmt> {
    // The hoisted `Let`s run before everything the loop runs: its condition on every pass, or its
    // bounds once, which may write a local too (an inlined call with a `var` argument).
    let mut written = HashSet::new();
    if let Stmt::ForRange { var, start, end, .. } = s {
        written.insert(*var);
        for e in [&*start, &*end] {
            walk_expr(e, &mut |n| note_write(n, &mut written));
        }
    }
    let (cond, body) = match s {
        Stmt::While { cond, body } => (Some(cond), body),
        Stmt::ForRange { body, .. } => (None, body),
        _ => unreachable!("not a loop"),
    };
    written.extend(self::written(body));
    if let Some(c) = &cond {
        walk_expr(c, &mut |n| note_write(n, &mut written));
    }
    let temps = temps(body, cond.as_deref(), &written, locals);
    let mut h = Hoist { written: &written, temps: &temps, locals, lets: Vec::new(), replaced };
    if let Some(c) = cond {
        h.visit_expr(c);
    }
    h.visit_stmts(body);
    h.lets
}

/// The locals `stmts` write: by `Let`, `Bind`, `BindEnv` or `BindGlobal`, as a `for` loop's
/// variable, or through any written place (an assignment, a `var` argument, a move).
fn written(stmts: &[Stmt]) -> HashSet<LocalIdx> {
    let mut out = HashSet::new();
    walk(stmts, &mut |n| note_write(n, &mut out));
    out
}

fn note_write(n: Node, out: &mut HashSet<LocalIdx>) {
    match n {
        Node::Stmt(Stmt::Let(l, _) | Stmt::Bind(l, _) | Stmt::BindGlobal(l, _) | Stmt::BindEnv { local: l, .. } | Stmt::ForRange { var: l, .. }) => {
            out.insert(*l);
        }
        Node::Place(p, true) => {
            out.insert(p.root);
        }
        _ => {}
    }
}

/// Whether `e` gives the same value on every pass of a loop that writes the locals in `written`,
/// and can neither fail nor change anything: literals, whole-local loads of `Value` locals the loop
/// never writes, casts to a float type, and float negation, arithmetic and scalar `Math`. Integer
/// arithmetic and casts to an integer are left out, since they may hold checks.
fn invariant(e: &Expr, written: &HashSet<LocalIdx>, locals: &[Local]) -> bool {
    invariant_by(e, &|l| locals[l.0 as usize].kind == LocalKind::Value && !written.contains(&l))
}

/// `invariant`, with `steady` saying which locals a load may read.
fn invariant_by(e: &Expr, steady: &impl Fn(LocalIdx) -> bool) -> bool {
    let float = matches!(e.ty, KTy::F32 | KTy::F64);
    match &e.kind {
        ExprKind::Float(_) | ExprKind::Int(_) | ExprKind::Bool(_) => true,
        ExprKind::Load(p) => p.path.is_empty() && steady(p.root),
        ExprKind::Cast(x) => float && is_scalar(x.ty) && invariant_by(x, steady),
        ExprKind::Unary(UnOp::Neg, x) => float && invariant_by(x, steady),
        ExprKind::Binary(BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Rem, a, b) => {
            float && invariant_by(a, steady) && invariant_by(b, steady)
        }
        ExprKind::Math(_, xs) => float && xs.iter().all(|x| invariant_by(x, steady)),
        _ => false,
    }
}

fn is_scalar(ty: KTy) -> bool {
    matches!(ty, KTy::Bool | KTy::U8 | KTy::I32 | KTy::I64 | KTy::F32 | KTy::F64)
}

/// The inliner's temporaries the loop body binds to invariant expressions (spec decision 5):
/// local -> initializer, for each `Value` local whose only writes in the loop are `Let`s at the
/// body's top level, each `invariant` by the base rule, before every use, with no use in a
/// `While`'s condition. One level: an initializer that reads another temporary doesn't count.
///
/// "Top level" is any `Let` every pass runs, in the order codegen evaluates the body (see
/// `Order`): a statement of the body itself, or one in a `Seq` that one of them evaluates, whose
/// local is used only inside that `Seq` (an inlined call used as a value, `s += scale(x, k)`,
/// binds its arguments there). That includes a `Seq` in the condition of an expression `if`,
/// which runs on every pass. Not inside an `if`'s branches (statement or expression), the second
/// operand of `and`/`or`, an inner loop, or a `Prove` or `Assume` block.
///
/// A use after such a `Let` runs after it in the same pass, and the local is written nowhere else
/// in the loop, so it reads the initializer's value, which is the same on every pass. Exactly
/// one `Let`: two would bind the local to two initializers.
fn temps(body: &[Stmt], cond: Option<&Expr>, written: &HashSet<LocalIdx>, locals: &[Local]) -> HashMap<LocalIdx, Expr> {
    let mut writes: HashMap<LocalIdx, usize> = HashMap::new();
    let mut count = |n: Node| {
        let mut one = HashSet::new();
        note_write(n, &mut one);
        for l in one {
            *writes.entry(l).or_default() += 1;
        }
    };
    walk(body, &mut count);
    let mut order = Order::default();
    if let Some(c) = cond {
        walk_expr(c, &mut count);
        // The condition runs before the body: a use there comes before any `Let`.
        order.branch += 1;
        order.visit_expr(&mut c.clone());
        order.branch -= 1;
    }
    order.visit_stmts(&mut body.to_vec());
    order
        .lets
        .into_iter()
        .filter(|(l, init)| {
            let local = &locals[l.0 as usize];
            writes.get(l) == Some(&1) && local.kind == LocalKind::Value && local.ty == init.ty && invariant(init, written, locals)
        })
        .collect()
}

/// Walks a loop body in the order codegen evaluates it (a copy: it changes nothing), collecting
/// each `Let` that every pass runs and that comes before any use of its local. A `Let` in a `Seq`
/// counts only when every use is inside that `Seq`, after it: a `Seq` runs its statements, then
/// its value, but where it runs among the other parts of the statement holding it is left to
/// codegen, so a use outside it might come first.
#[derive(Default)]
struct Order {
    /// How many conditional parts (branches, inner loops, blocks) the walk is inside.
    branch: u32,
    /// The locals of every place met so far.
    seen: HashSet<LocalIdx>,
    /// The locals bound by a `Let` of a `Seq` the walk has left.
    closed: HashSet<LocalIdx>,
    lets: HashMap<LocalIdx, Expr>,
}

impl Order {
    fn branch(&mut self, f: impl FnOnce(&mut Self)) {
        self.branch += 1;
        f(self);
        self.branch -= 1;
    }
}

impl VisitMut for Order {
    fn visit_stmt(&mut self, s: &mut Stmt) {
        match s {
            Stmt::Let(l, e) => {
                self.visit_expr(e);
                if self.branch == 0 && !self.seen.contains(l) {
                    self.lets.insert(*l, e.clone());
                }
            }
            Stmt::If { cond, then_body, else_body } => {
                self.visit_expr(cond);
                self.branch(|o| {
                    o.visit_stmts(then_body);
                    o.visit_stmts(else_body);
                });
            }
            Stmt::While { .. } | Stmt::ForRange { .. } | Stmt::Assume { .. } | Stmt::Prove { .. } => self.branch(|o| walk_stmt(o, s)),
            _ => walk_stmt(self, s),
        }
    }

    fn visit_expr(&mut self, e: &mut Expr) {
        match &mut e.kind {
            ExprKind::And(a, b) | ExprKind::Or(a, b) => {
                self.visit_expr(a);
                self.branch(|o| o.visit_expr(b));
            }
            // The condition runs on every pass, once; only the branches are conditional.
            ExprKind::If { cond, then_body, then_val, else_body, else_val } => {
                self.visit_expr(cond);
                self.branch(|o| {
                    o.visit_stmts(then_body);
                    then_val.iter_mut().for_each(|x| o.visit_expr(x));
                    o.visit_stmts(else_body);
                    else_val.iter_mut().for_each(|x| o.visit_expr(x));
                });
            }
            ExprKind::Seq(stmts, _) => {
                let bound: Vec<LocalIdx> = stmts.iter().filter_map(|s| if let Stmt::Let(l, _) = s { Some(*l) } else { None }).collect();
                walk_expr_mut(self, e);
                self.closed.extend(bound);
            }
            _ => walk_expr_mut(self, e),
        }
    }

    fn visit_place(&mut self, p: &mut Place) {
        walk_place(self, p);
        self.seen.insert(p.root);
        if self.closed.contains(&p.root) {
            self.lets.remove(&p.root);
        }
    }
}

/// Whether `e` holds a transcendental `Math` (`sin` through `pow`) or a float `%`: work that
/// Cranelift won't move out of a loop. (`sqrt`, `floor`, `abs` and the like are instructions it
/// hoists itself.)
fn costly(e: &Expr) -> bool {
    let mut found = false;
    walk_expr(e, &mut |n| {
        if let Node::Expr(x) = n {
            found |= match &x.kind {
                ExprKind::Math(op, _) => matches!(
                    op,
                    MathOp::Sin
                        | MathOp::Cos
                        | MathOp::Tan
                        | MathOp::Asin
                        | MathOp::Acos
                        | MathOp::Atan
                        | MathOp::Atan2
                        | MathOp::Exp
                        | MathOp::Log
                        | MathOp::Pow
                ),
                ExprKind::Binary(BinOp::Rem, ..) => matches!(x.ty, KTy::F32 | KTy::F64),
                _ => false,
            };
        }
    });
    found
}

/// Replaces each maximal hoistable expression in one loop's body and condition.
struct Hoist<'a> {
    written: &'a HashSet<LocalIdx>,
    temps: &'a HashMap<LocalIdx, Expr>,
    locals: &'a mut Vec<Local>,
    lets: Vec<Stmt>,
    /// Each new local, with the expression it replaced (before any temporary is seen through).
    replaced: &'a mut Vec<(LocalIdx, Expr)>,
}

impl VisitMut for Hoist<'_> {
    fn visit_stmt(&mut self, s: &mut Stmt) {
        match s {
            // An assumption's conditions stay as written: range analysis reads facts from them.
            Stmt::Assume { body, .. } => self.visit_stmts(body),
            _ => walk_stmt(self, s),
        }
    }

    fn visit_expr(&mut self, e: &mut Expr) {
        let (written, temps, locals) = (self.written, self.temps, &*self.locals);
        let steady = |l: LocalIdx| temps.contains_key(&l) || (locals[l.0 as usize].kind == LocalKind::Value && !written.contains(&l));
        if !(invariant_by(e, &steady) && costly(e)) {
            walk_expr_mut(self, e);
            return;
        }
        let mut value = e.clone();
        Substitute(temps).visit_expr(&mut value);
        self.locals.push(Local { name: HOISTED.into(), ty: e.ty, kind: LocalKind::Value });
        let l = LocalIdx(self.locals.len() as u32 - 1);
        self.lets.push(Stmt::Let(l, value));
        let load = Expr { kind: ExprKind::Load(Place::local(l)), ty: e.ty, line: e.line };
        self.replaced.push((l, std::mem::replace(e, load)));
    }
}

/// Puts each temporary's initializer in place of its loads.
struct Substitute<'a>(&'a HashMap<LocalIdx, Expr>);

impl VisitMut for Substitute<'_> {
    fn visit_expr(&mut self, e: &mut Expr) {
        if let ExprKind::Load(p) = &e.kind {
            if let Some(init) = p.path.is_empty().then(|| self.0.get(&p.root)).flatten() {
                *e = init.clone();
                return;
            }
        }
        walk_expr_mut(self, e);
    }
}
