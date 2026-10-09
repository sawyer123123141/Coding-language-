//! Moves of owning values and the arena escape check: which arena a value's
//! memory lives in (its region), and where such a value may be stored.

use kestrel_syntax::ast::{Block, Expr, ExprKind, Stmt};

use super::{BodyChecker, Expect};
use crate::tables::{LocalId, Res};
use crate::ty::Ty;

/// Where new memory goes at a point in the code.
#[derive(Clone, Debug)]
pub(crate) enum Ctx {
    /// The function's own: the program arena in `main`, the caller's
    /// current arena anywhere else.
    Function,
    /// `arena { }`: freed at its `}`.
    Scoped,
    /// `in x.mem { }`: belongs to `x`, as written.
    Owned(PathKey),
}

/// A place as written: its root variable (an alias stays itself) and its
/// steps, `Some(field)` or `None` for an index.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct PathKey {
    root: LocalId,
    steps: Vec<Option<String>>,
}

impl PathKey {
    /// Whether `self` is `p` or inside it. An index in `p` matches nothing,
    /// since two indexes may differ.
    fn starts_with(&self, p: &PathKey) -> bool {
        self.root == p.root
            && p.steps.len() <= self.steps.len()
            && p.steps.iter().zip(&self.steps).all(|(a, b)| a.is_some() && a == b)
    }
}

/// A set of contexts, by index into `BodyChecker::ctxs`.
fn bit(i: usize) -> u64 {
    1 << i.min(63)
}

fn tail(block: &Block) -> Option<&Expr> {
    match block.stmts.last() {
        Some(Stmt::Expr(e)) => Some(e),
        _ => None,
    }
}

impl<'a> BodyChecker<'a> {
    pub(crate) fn path_key(&self, e: &Expr) -> Option<PathKey> {
        match &e.kind {
            ExprKind::Ident(_) => match self.t.res.get(&e.id) {
                Some(Res::Local(l)) => Some(PathKey { root: *l, steps: Vec::new() }),
                _ => None,
            },
            ExprKind::Field { .. } if self.t.swizzles.contains_key(&e.id) => None,
            ExprKind::Field { base, name, .. } => {
                let mut p = self.path_key(base)?;
                p.steps.push(Some(name.clone()));
                Some(p)
            }
            ExprKind::Index { base, .. } => {
                let mut p = self.path_key(base)?;
                p.steps.push(None);
                Some(p)
            }
            _ => None,
        }
    }

    fn path_name(&self, p: &PathKey) -> String {
        let mut s = self.t.local(p.root).name.clone();
        for step in &p.steps {
            match step {
                Some(f) => s.push_str(&format!(".{f}")),
                None => s.push_str("[_]"),
            }
        }
        s
    }

    fn current(&self) -> u64 {
        bit(self.ctxs.len() - 1)
    }

    fn is_main(&self) -> bool {
        self.fn_id.is_some() && self.fn_id == self.t.main
    }

    /// The contexts whose memory the value of `e` points into. A copy of a
    /// place goes to the current arena. With `moving` (a returned value),
    /// reads of locals aren't copied, so they keep their local's context.
    fn region(&self, e: &Expr, moving: bool) -> u64 {
        if !self.t.has_heap(self.t.ty(e.id)) {
            return 0;
        }
        match &e.kind {
            ExprKind::ArrayLit(items) if items.is_empty() => 0,
            ExprKind::Field { .. } if matches!(self.t.res.get(&e.id), Some(Res::Variant(..))) => 0,
            ExprKind::Ident(_) | ExprKind::Field { .. } | ExprKind::Index { .. } => {
                let local = match (moving || self.t.moves.contains(&e.id), self.place_root(e)) {
                    (true, Some(root)) if !self.t.local(root).is_param => self.local_ctx.get(&root).map(|&(c, _)| bit(c)),
                    _ => None,
                };
                // A view (a loop element) may be copied or not; assume either.
                let view = self.path_key(e).is_some_and(|p| self.views.contains(&p.root));
                match local {
                    Some(r) if view => r | self.current(),
                    Some(r) => r,
                    None => self.current(),
                }
            }
            ExprKind::Call { args, .. } | ExprKind::MethodCall { args, .. }
                if matches!(self.t.res.get(&e.id), Some(Res::Construct(_) | Res::Variant(..))) =>
            {
                args.iter().fold(0, |r, a| r | self.region(&a.value, moving))
            }
            ExprKind::If { then_block, else_block, .. } => {
                let blocks = std::iter::once(then_block).chain(else_block.as_ref());
                blocks.filter_map(tail).fold(0, |r, t| r | self.region(t, moving))
            }
            ExprKind::Match { arms, .. } => arms.iter().filter_map(|a| tail(&a.body)).fold(0, |r, t| r | self.region(t, moving)),
            _ => self.current(),
        }
    }

    /// Checks that the memory `value` points into lives at least as long as
    /// the place `target` it's stored into.
    pub(crate) fn check_store(&mut self, target: &Expr, value: &Expr) {
        let r = self.region(value, false);
        let Some(root) = self.place_root(target) else { return };
        let path = self.path_key(target);
        let info = self.t.local(root);
        let name = info.name.clone();
        // Parameters outlive the function, so `None`.
        let decl = if info.is_param { None } else { self.local_ctx.get(&root).map(|&(c, _)| c) };
        for i in (0..self.ctxs.len()).filter(|&i| r & bit(i) != 0) {
            let (ok, msg, help) = match &self.ctxs[i] {
                Ctx::Function => (
                    self.is_main() || decl.is_some(),
                    format!("this memory is in the caller's current arena, which may not live as long as `{name}`"),
                    format!("allocate it with `in {name}.mem {{ }}`"),
                ),
                Ctx::Scoped => (
                    decl.is_some_and(|d| d >= i),
                    format!("this memory is freed at the end of the `arena {{ }}` block, but `{name}` lives longer"),
                    "build it outside the block, or store it in a variable declared inside".to_string(),
                ),
                Ctx::Owned(p) => {
                    let owner = self.path_name(p);
                    (
                        decl.is_some_and(|d| d >= i) || path.as_ref().is_some_and(|t| t.starts_with(p)),
                        format!("this memory belongs to `{owner}`, but `{name}` isn't part of `{owner}`"),
                        format!("store it inside `{owner}`, or in a variable declared in this `in` block"),
                    )
                }
            };
            if !ok {
                return self.error_help(value.span, msg, help);
            }
        }
    }

    /// `in x.mem { }`: allocations inside belong to `x`.
    pub(crate) fn check_in(&mut self, arena: &Expr, body: &Block) -> Ty {
        let ty = self.check_expr(arena, None);
        let owner = match &arena.kind {
            ExprKind::Field { base, .. } => self.path_key(base),
            _ => self.path_key(arena),
        };
        let ctx = match (ty, owner) {
            (Ty::Arena, Some(p)) => Some(Ctx::Owned(p)),
            (Ty::Arena, None) => {
                self.error(arena.span, "`in` needs an arena stored in a variable or field");
                None
            }
            (Ty::Error, _) => None,
            (other, _) => {
                let msg = format!("`in` needs an `Arena`, like `in self.mem {{ }}`; found {}", self.tn(other));
                self.error(arena.span, msg);
                None
            }
        };
        let pushed = ctx.is_some();
        self.ctxs.extend(ctx);
        let ty = self.check_block(body, Expect::Discard);
        if pushed {
            self.ctxs.pop();
        }
        if ty == Ty::Never {
            Ty::Never
        } else {
            Ty::Unit
        }
    }

    /// A returned value can only point into the caller's memory.
    pub(crate) fn check_return_region(&mut self, value: &Expr) {
        let r = self.region(value, true) & !bit(0);
        let Some(i) = (1..self.ctxs.len()).find(|&i| r & bit(i) != 0) else { return };
        let msg = match &self.ctxs[i] {
            Ctx::Owned(p) => format!("can't return memory allocated with `in`: it belongs to `{}`", self.path_name(p)),
            _ => "can't return memory from an `arena { }` block: it's freed at the block's end".to_string(),
        };
        self.error(value.span, msg);
    }

    /// `v` is stored somewhere new. An owning value (one holding an `Arena`)
    /// can't be copied, only moved, and only whole variables move.
    pub(crate) fn consume(&mut self, v: &Expr) {
        if !self.t.is_owning(self.t.ty(v.id)) {
            return;
        }
        match &v.kind {
            ExprKind::If { then_block, else_block, .. } => {
                for b in std::iter::once(then_block).chain(else_block.as_ref()) {
                    if let Some(t) = tail(b) {
                        self.consume(t);
                    }
                }
            }
            ExprKind::Match { arms, .. } => {
                for a in arms {
                    if let Some(t) = tail(&a.body) {
                        self.consume(t);
                    }
                }
            }
            ExprKind::Ident(_) => {
                let Some(&Res::Local(l)) = self.t.res.get(&v.id) else { return };
                let info = self.t.local(l);
                let name = info.name.clone();
                let loops = self.local_ctx.get(&l).map_or(0, |&(_, loops)| loops);
                if info.is_param {
                    let msg = format!("can't move parameter `{name}`: it belongs to the caller, and it holds an `Arena`, so it can't be copied");
                    self.error(v.span, msg);
                } else if self.views.contains(&l) {
                    let msg = format!("can't move `{name}` out of its collection: it holds an `Arena`, so it can't be copied");
                    self.error(v.span, msg);
                } else if self.loop_breaks.len() > loops {
                    let msg = format!("`{name}` is moved inside a loop, so the next time around it would already be gone");
                    self.error_help(v.span, msg, "declare it inside the loop");
                } else {
                    self.moved.insert(l);
                    self.t.moves.insert(v.id);
                }
            }
            ExprKind::Field { .. } | ExprKind::Index { .. } if self.place_root(v).is_some() => {
                self.error_help(
                    v.span,
                    "can't move this out: it holds an `Arena`, so it can't be copied, and only whole variables move",
                    "move the whole variable that holds it",
                );
            }
            _ => {}
        }
    }
}
