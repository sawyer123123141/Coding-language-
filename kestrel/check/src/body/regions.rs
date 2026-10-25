//! The arena escape check: which arena a value's memory lives in (its region), and where
//! such a value may be stored. Moves are in `moves`.

use kestrel_syntax::ast::{Block, Expr, ExprKind, Stmt};

use super::BodyChecker;
use crate::tables::{ListMethod, LocalId, Res};
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
    pub(super) steps: Vec<Option<String>>,
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
pub(super) fn bit(i: usize) -> u64 {
    1 << i.min(63)
}

/// A place a store goes to: its root variable (aliases followed), its path as written, the
/// context it was declared in (`None` for a parameter, which outlives the function), and the
/// variable as written, for messages.
pub(super) struct StoreTarget {
    pub(super) root: LocalId,
    path: Option<PathKey>,
    decl: Option<usize>,
    pub(super) name: String,
    pub(super) written_ty: Ty,
}

pub(crate) fn tail(block: &Block) -> Option<&Expr> {
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

    pub(super) fn current(&self) -> u64 {
        bit(self.ctxs.len() - 1)
    }

    fn is_main(&self) -> bool {
        // A closure body runs in the arena current where it is called, even one written in `main`.
        self.frames.is_empty() && self.fn_id.is_some() && self.fn_id == self.t.main
    }

    /// The contexts whose memory the value of `e` points into. A copy of a
    /// place goes to the current arena. With `moving` (a returned value),
    /// reads of locals aren't copied, so they keep their local's context,
    /// unless the local is owning (it holds an `Arena` or a GPU object): an arena
    /// it holds may hold the value's memory and is dropped on the way out, so
    /// the value is copied.
    pub(super) fn region(&self, e: &Expr, moving: bool) -> u64 {
        let inner = self.region_of_value(e, moving);
        // A value stored as an interface value is copied into the current arena whatever it is, so
        // a struct of plain numbers has a region once it is one (like a string).
        if self.boxes_iface(e) {
            inner | self.current()
        } else {
            inner
        }
    }

    /// `region` for the value `e` has before any conversion to an interface value.
    fn region_of_value(&self, e: &Expr, moving: bool) -> u64 {
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
            // A closure that captures nothing has no environment, like a named function; one that
            // captures is built with its environment in the current arena.
            ExprKind::Closure(_) => {
                if self.t.closures.get(&e.id).is_some_and(|c| !c.captures.is_empty()) {
                    self.current()
                } else {
                    0
                }
            }
            // A named function has no environment, so it points into no arena.
            ExprKind::Ident(_) | ExprKind::TypeApply { .. } if matches!(self.t.res.get(&e.id), Some(Res::FnValue(_))) => 0,
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
            // A character is a new string, built in the current arena.
            ExprKind::Index { base, .. } if self.t.ty(base.id) == Ty::Str => self.current(),
            ExprKind::Ident(_) | ExprKind::Field { .. } | ExprKind::Index { .. } => {
                let ctx = |root: LocalId| self.local_ctx.get(&root).map(|&(c, _)| bit(c));
                let local = match self.place_root(e) {
                    Some(root) if self.t.moves.contains(&e.id) => ctx(root),
                    // Inside a closure's body, a capture belongs to the caller, like a parameter.
                    Some(root) if moving && !self.t.local(root).is_param && !self.is_capture(root) && !self.t.is_owning(self.t.local(root).ty) => {
                        ctx(root)
                    }
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
            // (A written construction, `Stack[int]()`, is the `Call` it stands for.)
            ExprKind::Call { args, .. } | ExprKind::MethodCall { args, .. } | ExprKind::TypeCall { args, .. }
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
            // `pop` moves an owning item out of the list's memory: it lives as long as the list,
            // not in the current arena. (A list that is a loop's item may be either, like a view.)
            // Any other item is copied into the current arena, like an indexed read.
            ExprKind::MethodCall { receiver, .. }
                if matches!(self.t.res.get(&e.id), Some(Res::List(ListMethod::Pop))) && self.t.is_owning(ty) =>
            {
                let ctx = self.place_root(receiver).and_then(|root| self.local_ctx.get(&root)).map(|&(c, _)| bit(c));
                let view = self.path_key(receiver).is_some_and(|p| self.views.contains(&p.root));
                match ctx {
                    Some(r) if view => r | self.current(),
                    Some(r) => r,
                    None => self.current(),
                }
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
        self.check_store_region(target, value, r);
    }

    /// `check_store` for a value that is built new in the current arena whatever `value` is:
    /// `s += t` (its `value` is `t`, which is only read).
    pub(crate) fn check_store_new(&mut self, target: &Expr, value: &Expr) {
        let r = self.current();
        self.check_store_region(target, value, r);
    }

    fn check_store_region(&mut self, target: &Expr, value: &Expr, r: u64) {
        let Some(t) = self.store_target(target) else { return };
        for i in (0..self.ctxs.len()).filter(|&i| r & bit(i) != 0) {
            if self.ctx_holds(i, &t) {
                continue;
            }
            // Caller memory stored into this function's own `var` parameter is checked at each call (P38).
            if matches!(self.ctxs[i], Ctx::Function) && self.defer_var_store(t.root) {
                continue;
            }
            let (name, written_ty) = (&t.name, t.written_ty);
            let (msg, help) = match self.ctxs[i].clone() {
                Ctx::Function => (
                    format!("this memory is in the caller's current arena, which may not live as long as `{name}`"),
                    self.caller_memory_help(name, written_ty),
                ),
                Ctx::Scoped => (
                    format!("this memory is freed at the end of the `arena {{ }}` block, but `{name}` lives longer"),
                    "build it outside the block, or store it in a variable declared inside".to_string(),
                ),
                Ctx::Scratch(callee) => (
                    format!("this memory is freed when the call to `{callee}` returns, but `{name}` lives longer"),
                    "build it before the call, or store it in a variable declared inside the argument".to_string(),
                ),
                Ctx::Owned(p) => {
                    let owner = self.path_name(&p);
                    (
                        format!("this memory belongs to `{owner}`, but `{name}` isn't part of `{owner}`"),
                        format!("store it inside `{owner}`, or in a variable declared in this `in` block"),
                    )
                }
            };
            return self.error_help(value.span, msg, help);
        }
    }

    /// The place a store goes to, as `ctx_holds` judges it. `None` when `target` isn't a place.
    pub(super) fn store_target(&self, target: &Expr) -> Option<StoreTarget> {
        let root = self.place_root(target)?;
        let path = self.path_key(target);
        let info = self.t.local(root);
        // Name the variable as written (a loop element, not its collection).
        let written = self.t.local(path.as_ref().map_or(root, |p| p.root));
        let (name, written_ty) = (written.name.clone(), written.ty);
        // Parameters outlive the function, so `None`.
        let decl = if info.is_param { None } else { self.local_ctx.get(&root).map(|&(c, _)| c) };
        Some(StoreTarget { root, path, decl, name, written_ty })
    }

    /// Whether memory of context `i` may be stored into `t`. The one test for a store written
    /// here and for a store a called function makes into its `var` parameter (see `call_stores`).
    pub(super) fn ctx_holds(&self, i: usize, t: &StoreTarget) -> bool {
        match &self.ctxs[i] {
            Ctx::Function => self.is_main() || t.decl.is_some(),
            Ctx::Scoped | Ctx::Scratch(_) => t.decl.is_some_and(|d| d >= i),
            Ctx::Owned(p) => t.decl.is_some_and(|d| d >= i) || t.path.as_ref().is_some_and(|path| path.starts_with(p)),
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
}
