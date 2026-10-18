//! The call exclusivity rule (the closure spec's "When captures are copied", under
//! "Exclusivity", and "Calling").
//!
//! A closure literal that is directly an argument of a call to a user function or method, or of a
//! call through a function value, gets a **stack environment** (`Tables::stack_closures`): shallow
//! copies of its captured values, taken when the argument is evaluated, which share heap memory
//! (a string's bytes, a list's items) with the caller's variables. So while the call runs, nothing
//! it can change may touch what such a literal captures. The same goes for the place a call
//! through a function value reads the value from: the value's environment may live there.
//!
//! **The changes** (`Changes`) are everything the call can change: its `var` arguments, a
//! `var self` receiver, the places changed anywhere inside its arguments (an assignment in an
//! `if`, a `var` argument of a call inside an argument), and the values moved into the call or
//! into any of its arguments. An owning value passed to a parameter that isn't `var` counts as
//! moved into the call (Kestrel passes it by pointer today, but the spec treats it as a move, and
//! the callee may hold the only owner of its memory while it runs).
//!
//! **A capture conflicts with a change** when their places overlap (`PathKey::overlaps`, with
//! views such as a loop item expanded to the place they read), or when the change is of owning
//! type (it holds an `Arena` or a GPU object), or a move, under the capture's root variable: an
//! `Arena` in a sibling field may hold the captured string's bytes. A local declared inside
//! `in w.mem { }` may hold memory of `w.mem`, so `w` counts as one of its places too.

use kestrel_syntax::ast::{Arg, Expr};
use kestrel_syntax::Span;

use super::regions::{Ctx, PathKey};
use super::BodyChecker;
use crate::tables::LocalId;

/// What a call can change, for the exclusivity rule.
#[derive(Default)]
pub(crate) struct Changes {
    /// The places changed: views expanded to the place they read, so each root is a variable.
    pub(crate) paths: Vec<PathKey>,
    /// The roots of the changes whose places are of owning type (or hold an owning value).
    pub(crate) owning_roots: Vec<LocalId>,
    /// The variables moved into the call or into its arguments.
    pub(crate) moved_roots: Vec<LocalId>,
}

/// How a method call changes its receiver, for the exclusivity rule.
#[derive(Clone, Copy, PartialEq)]
pub(crate) enum RecvChange {
    /// Not at all.
    Read,
    /// Any way: a `var self` receiver.
    Changed,
    /// Only the order of its items: the list a `sort` or `sort_by` sorts.
    Reordered,
}

/// The place a call through a function value reads the value from (views expanded), and the
/// callee as the program writes it, for the message.
pub(crate) struct CalleePlace {
    pub(crate) key: PathKey,
    pub(crate) shown: String,
}

impl<'a> BodyChecker<'a> {
    /// Records `a` as a closure literal with a stack environment, when it is one. Called for an
    /// argument of a user function, a user method or a function value, before it is checked.
    pub(crate) fn note_stack_closure(&mut self, a: &Arg) {
        if matches!(a.value.kind, kestrel_syntax::ast::ExprKind::Closure(_)) {
            self.t.stack_closures.insert(a.value.id);
        }
    }

    /// The place `e` names, with views expanded to the place they read (a loop item `b` of
    /// `for b in w.bricks` is `w.bricks[_]`); all of its variable when its steps can't be told.
    /// Call after `e` has been checked.
    pub(crate) fn source_key(&self, e: &Expr) -> Option<PathKey> {
        match self.list_place(e) {
            Some((root, steps)) => Some(PathKey { root, steps }),
            None => self.place_root(e).map(|root| PathKey { root, steps: Vec::new() }),
        }
    }

    /// The place `local` stands for, with a view expanded to the place it reads.
    pub(crate) fn local_key(&self, local: LocalId) -> PathKey {
        let (root, steps) = self.view_source(local);
        PathKey { root, steps }
    }

    /// `target` is changed: its place, and its root when its type is owning.
    pub(crate) fn note_changed(&self, target: &Expr, out: &mut Changes) {
        let Some(key) = self.source_key(target) else { return };
        if self.t.is_owning(self.t.ty(target.id)) {
            out.owning_roots.push(key.root);
        }
        out.paths.push(key);
    }

    /// `value` is passed to a parameter that isn't `var`: an owning variable (or a part of one)
    /// counts as moved into the call.
    pub(crate) fn note_passed(&self, value: &Expr, out: &mut Changes) {
        if self.t.is_owning(self.t.ty(value.id)) {
            out.moved_roots.extend(self.place_root(value));
        }
    }

    /// What a call with these arguments can change. `receiver` is the receiver of a method call,
    /// with how the method changes it.
    pub(crate) fn call_changes(&self, args: &[Arg], receiver: Option<(&Expr, RecvChange)>) -> Changes {
        let mut out = Changes::default();
        if let Some((r, how)) = receiver {
            match how {
                RecvChange::Changed => self.note_changed(r, &mut out),
                // Sorting moves the items as bytes and frees nothing, so an owning item doesn't
                // make every sibling of the list a conflict: only the list's own place is one.
                RecvChange::Reordered => out.paths.extend(self.source_key(r)),
                RecvChange::Read => {}
            }
            self.changes_in(r, &mut out);
        }
        self.changes_by_args(args, &mut out);
        out
    }

    /// Checks the captures of each stack-environment literal among `args`, and the place
    /// `callee_place` a call through a function value reads its value from, against what the call
    /// changes. Call after the arguments have been checked.
    pub(crate) fn check_call_captures(&mut self, call_span: Span, args: &[Arg], changes: &Changes, callee_place: Option<CalleePlace>) {
        for a in args {
            if !self.t.stack_closures.contains(&a.value.id) {
                continue;
            }
            let Some(info) = self.t.closures.get(&a.value.id) else { continue };
            let captures = info.captures.clone();
            // One error per literal: the first capture that conflicts.
            let found = captures.iter().find_map(|c| {
                let key = PathKey { root: c.source_root, steps: c.source_path.clone() };
                self.conflict(&key, changes).map(|name| (name, c.span))
            });
            if let Some((name, span)) = found {
                self.error(span, format!("`{name}` is changed by this call, so the closure passed to it can't read it"));
            }
        }
        if let Some(callee) = callee_place {
            if let Some(name) = self.conflict(&callee.key, changes) {
                let msg = format!("can't call `{}` while this call changes `{name}`, which may hold its memory", callee.shown);
                self.error(call_span, msg);
            }
        }
    }

    /// The change that conflicts with reading `key` (or the arenas that may hold its memory), named
    /// for the message: the changed place as written up to its first index, or the variable for
    /// an owning change or a move.
    fn conflict(&self, key: &PathKey, changes: &Changes) -> Option<String> {
        let mut keys = vec![key.clone()];
        keys.extend(self.arena_owners(key.root));
        for k in &keys {
            if let Some(p) = changes.paths.iter().find(|p| k.overlaps(p)) {
                return Some(self.change_name(p));
            }
            if changes.owning_roots.contains(&k.root) || changes.moved_roots.contains(&k.root) {
                return Some(self.t.local(k.root).name.clone());
            }
        }
        None
    }

    /// The owners of the `in x.mem { }` arenas whose memory the variable `root` may hold: those
    /// around its declaration (`check_store` refuses memory from a block it was declared outside).
    fn arena_owners(&self, root: LocalId) -> Vec<PathKey> {
        if self.t.local(root).is_param {
            return Vec::new();
        }
        let Some(&(decl, _)) = self.local_ctx.get(&root) else { return Vec::new() };
        let upto = decl.min(self.ctxs.len().saturating_sub(1));
        self.ctxs[..=upto]
            .iter()
            .filter_map(|c| match c {
                Ctx::Owned(p) => {
                    let mut key = self.local_key(p.root);
                    key.steps.extend(p.steps.iter().cloned());
                    Some(key)
                }
                _ => None,
            })
            .collect()
    }

    /// A changed place as a message writes it: its variable and fields, up to the first index.
    fn change_name(&self, p: &PathKey) -> String {
        let mut name = self.t.local(p.root).name.clone();
        for step in p.steps.iter().map_while(Option::as_ref) {
            name.push('.');
            name.push_str(step);
        }
        name
    }
}
