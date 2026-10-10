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
    /// The arguments of a call to this native (a display name like `draw.text`),
    /// when one is an interpolated string: built in a scratch arena freed
    /// when the call returns.
    Scratch(String),
}

/// A place as written: its root variable (an alias stays itself) and its
/// steps, `Some(field)` or `None` for an index.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct PathKey {
    pub(crate) root: LocalId,
    steps: Vec<Option<String>>,
}

impl PathKey {
    /// Whether `self` and `p` may name overlapping memory: one is inside
    /// the other, with an index matching any index.
    pub(crate) fn overlaps(&self, p: &PathKey) -> bool {
        self.root == p.root && self.steps.iter().zip(&p.steps).all(|(a, b)| a.is_none() || b.is_none() || a == b)
    }

    /// Whether `self` is `p` or inside it. An index in `p` matches nothing,
    /// since two indexes may differ.
    /// Whether `self` is `p` with more steps after (or `p` itself), steps
    /// compared as written.
    pub(crate) fn extends(&self, p: &PathKey) -> bool {
        self.root == p.root && p.steps.len() <= self.steps.len() && p.steps.iter().zip(&self.steps).all(|(a, b)| a == b)
    }

    fn starts_with(&self, p: &PathKey) -> bool {
        self.root == p.root
            && p.steps.len() <= self.steps.len()
            && p.steps.iter().zip(&self.steps).all(|(a, b)| a.is_some() && a == b)
    }
}

/// A `match` arm with a binding: the matched place and the binding.
#[derive(Clone)]
pub(crate) struct MatchGuard {
    pub(crate) root: LocalId,
    pub(crate) path: Option<PathKey>,
    pub(crate) binding: LocalId,
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

    pub(crate) fn path_name(&self, p: &PathKey) -> String {
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
    /// reads of locals aren't copied, so they keep their local's context,
    /// unless the local owns an arena: its memory may live in that arena,
    /// which is dropped on the way out, so it's copied.
    fn region(&self, e: &Expr, moving: bool) -> u64 {
        let ty = self.t.ty(e.id);
        if !self.t.has_heap(ty) {
            return 0;
        }
        match &e.kind {
            ExprKind::None => 0,
            // Literals live in read-only data and belong to no arena. A string
            // constant is a literal, built again at each use.
            ExprKind::Str(_) => 0,
            ExprKind::Ident(_) if ty == Ty::Str && matches!(self.t.res.get(&e.id), Some(Res::Const(_))) => 0,
            // An empty list still remembers the current arena, to grow in.
            ExprKind::ArrayLit(items) if items.is_empty() => {
                if matches!(ty, Ty::List(_)) {
                    self.current()
                } else {
                    0
                }
            }
            // Moved elements bring their memory along.
            ExprKind::ArrayLit(items) => items.iter().fold(self.current(), |r, i| r | self.region(i, moving)),
            ExprKind::Field { .. } if matches!(self.t.res.get(&e.id), Some(Res::Variant(..))) => 0,
            ExprKind::Ident(_) | ExprKind::Field { .. } | ExprKind::Index { .. } => {
                let ctx = |root: LocalId| self.local_ctx.get(&root).map(|&(c, _)| bit(c));
                let local = match self.place_root(e) {
                    Some(root) if self.t.moves.contains(&e.id) => ctx(root),
                    Some(root) if moving && !self.t.local(root).is_param && !self.t.is_owning(self.t.local(root).ty) => ctx(root),
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
                // Defaults like `[1, 2]` or `list[T]()` allocate here too.
                let defaults = match self.t.res.get(&e.id) {
                    Some(Res::Construct(sid)) => {
                        let fields = &self.t.strukt(*sid).fields;
                        let slots = self.t.ctor_args.get(&e.id);
                        slots.is_some_and(|s| s.iter().zip(fields).any(|(slot, f)| slot.is_none() && f.default_allocates))
                    }
                    _ => false,
                };
                let start = if defaults { self.current() } else { 0 };
                args.iter().fold(start, |r, a| r | self.region(&a.value, moving))
            }
            ExprKind::If { then_block, else_block, .. } => {
                let blocks = std::iter::once(then_block).chain(else_block.as_ref());
                blocks.filter_map(tail).fold(0, |r, t| r | self.region(t, moving))
            }
            ExprKind::Match { arms, .. } => arms.iter().filter_map(|a| tail(&a.body)).fold(0, |r, t| r | self.region(t, moving)),
            _ => self.current(),
        }
    }

    /// What to do about memory in the caller's current arena that is stored
    /// into `name`, a variable of type `ty`: build it in an arena that `name`
    /// owns, which takes an `Arena` field on its type.
    fn caller_memory_help(&self, name: &str, ty: Ty) -> String {
        let (owner, fields) = match ty {
            Ty::Struct(s) => (format!("`{}`", self.t.strukt(s).name), &self.t.strukt(s).fields),
            Ty::Variant(e, v) => {
                let variant = &self.t.enm(e).variants[v as usize];
                (format!("the `{}` variant", variant.name), &variant.fields)
            }
            _ => {
                return format!(
                    "build it in the caller, or give the owner of `{name}` an `Arena` field and build it inside `in <owner>.<that field> {{ }}`"
                )
            }
        };
        match fields.iter().find(|f| f.ty == Ty::Arena) {
            Some(arena) => format!("allocate it with `in {name}.{} {{ }}`", arena.name),
            None => format!("give {owner} an `Arena` field, and build it inside `in {name}.<that field> {{ }}`"),
        }
    }

    /// Checks that the memory `value` points into lives at least as long as
    /// the place `target` it's stored into.
    pub(crate) fn check_store(&mut self, target: &Expr, value: &Expr) {
        let r = self.region(value, false);
        let Some(root) = self.place_root(target) else { return };
        let path = self.path_key(target);
        let info = self.t.local(root);
        // Name the variable as written (a loop element, not its collection).
        let written = self.t.local(path.as_ref().map_or(root, |p| p.root));
        let (name, written_ty) = (written.name.clone(), written.ty);
        // Parameters outlive the function, so `None`.
        let decl = if info.is_param { None } else { self.local_ctx.get(&root).map(|&(c, _)| c) };
        for i in (0..self.ctxs.len()).filter(|&i| r & bit(i) != 0) {
            let (ok, msg, help) = match &self.ctxs[i] {
                Ctx::Function => (
                    self.is_main() || decl.is_some(),
                    format!("this memory is in the caller's current arena, which may not live as long as `{name}`"),
                    self.caller_memory_help(&name, written_ty),
                ),
                Ctx::Scoped => (
                    decl.is_some_and(|d| d >= i),
                    format!("this memory is freed at the end of the `arena {{ }}` block, but `{name}` lives longer"),
                    "build it outside the block, or store it in a variable declared inside".to_string(),
                ),
                Ctx::Scratch(callee) => (
                    decl.is_some_and(|d| d >= i),
                    format!("this memory is freed when the call to `{callee}` returns, but `{name}` lives longer"),
                    "build it before the call, or store it in a variable declared inside the argument".to_string(),
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
            (Ty::Arena, Some(p)) => {
                self.check_changeable_arena(arena);
                self.check_par_write(arena);
                Some(Ctx::Owned(p))
            }
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

    /// Allocating into an arena changes it, so its owner must be changeable.
    fn check_changeable_arena(&mut self, arena: &Expr) {
        let Some(root) = self.place_root(arena) else { return };
        let name = self.t.local(root).name.clone();
        let help = match self.immutable(root) {
            None => return self.mark_changed(root),
            Some(super::place::Immutable::Let) => format!("declare `{name}` with `var`"),
            Some(_) => format!("take `{name}` as a `var` parameter"),
        };
        self.error_help(arena.span, format!("`in` allocates into this arena, so `{name}` must be changeable"), help);
    }

    /// While a `match` arm's binding reads from a place, nothing may change
    /// that place (or anything holding it) except through the binding:
    /// the binding would then read a different variant's fields.
    pub(crate) fn check_not_matched(&mut self, target: &Expr) {
        let Some(root) = self.place_root(target) else { return };
        let Some(path) = self.path_key(target) else { return };
        for g in self.match_guards.clone() {
            if path.root == g.binding || root != g.root {
                continue;
            }
            let overlaps = match &g.path {
                Some(s) if s.root == path.root => {
                    path.steps.len() <= s.steps.len()
                        && path.steps.iter().zip(&s.steps).all(|(a, b)| a.is_none() || b.is_none() || a == b)
                }
                _ => true,
            };
            if overlaps {
                let (name, binding) = (self.t.local(root).name.clone(), self.t.local(g.binding).name.clone());
                return self.error_help(
                    target.span,
                    format!("can't change `{name}` here: this `match` arm's `{binding}` reads from it"),
                    format!("change it after the `match`, or copy what you need out of `{binding}` first"),
                );
            }
        }
    }

    /// A returned value can only point into the caller's memory.
    pub(crate) fn check_return_region(&mut self, value: &Expr) {
        let r = self.region(value, true) & !bit(0);
        let Some(i) = (1..self.ctxs.len()).find(|&i| r & bit(i) != 0) else { return };
        let msg = match &self.ctxs[i] {
            Ctx::Owned(p) => format!("can't return memory allocated with `in`: it belongs to `{}`", self.path_name(p)),
            Ctx::Scratch(callee) => format!("can't return memory from inside a call to `{callee}`: it's freed when the call returns"),
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
