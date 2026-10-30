//! Changing a `for` loop's own variables: its counter, index, key or string character can't
//! change, and its item changes the list or map it reads, so that must be a `var`. Each gets a
//! message of its own: "declared with `let`" and "declare it with `var`" would point at `for var`,
//! which isn't Kestrel.

use kestrel_syntax::ast::{Expr, ExprKind};

use super::place::Immutable;
use super::BodyChecker;
use crate::tables::{LocalId, Res};

/// What a loop declares a local as.
#[derive(Clone, Copy)]
pub(crate) enum LoopVar {
    /// `i` in `for i from 0 to n`.
    Counter,
    /// `i` in `for i, x in xs` (over a list, an array or a string).
    Index,
    /// `k` in `for k, v in m` or `for k in m`.
    Key,
    /// `c` in `for c in s`.
    Char,
    /// `x` in `for x in xs` (or `v` in `for k, v in m`, with `map`), changing the item it reads.
    /// `stored` is whether the loop reads a variable (otherwise a value built for the loop).
    Item { stored: bool, map: bool },
}

impl<'a> BodyChecker<'a> {
    pub(crate) fn note_loop_var(&mut self, local: LocalId, kind: LoopVar) {
        self.loop_vars.insert(local, kind);
    }

    /// An assignment to `target`, which can't change: when the variable written is one of a
    /// loop's own, reports why and returns true.
    pub(crate) fn loop_var_assign_error(&mut self, target: &Expr) -> bool {
        let Some((name, why, help)) = self.loop_var_reason(target) else { return false };
        self.error_help(target.span, format!("can't change `{name}`: {why}"), help);
        true
    }

    /// For a place `e` that can't change, written on one of a loop's own variables: that
    /// variable's name, why it can't change, and what to do. The caller says what was refused
    /// ("can't change `i`", "can't pass `i` as `var`", "can't call `grow` on `p`").
    pub(crate) fn loop_var_reason(&self, e: &Expr) -> Option<(String, String, String)> {
        let local = self.written_local(e)?;
        let &kind = self.loop_vars.get(&local)?;
        let name = self.t.local(local).name.clone();
        let changes_only_with = |what: &str| (format!("a `for` loop's {what} changes only with the loop"), "count with a `while` loop to step it yourself".to_string());
        let (why, help) = match kind {
            LoopVar::Counter => changes_only_with("counter"),
            LoopVar::Index => changes_only_with("index"),
            LoopVar::Key => ("a map loop's key can't be changed".to_string(), "copy it into a `var` first".to_string()),
            LoopVar::Char => {
                ("a `for` loop over a string reads its characters, which can't be changed".to_string(), "build a new string instead".to_string())
            }
            LoopVar::Item { stored: false, .. } => (
                "the loop's items come from a value that isn't stored in a variable".to_string(),
                "store it in a `var` first and loop over that".to_string(),
            ),
            LoopVar::Item { stored: true, map } => {
                let root = self.t.local(local).alias?;
                let list = self.t.local(root).name.clone();
                let of = if map { "a value in" } else { "an item of" };
                match self.immutable(root) {
                    Some(Immutable::Let) => (format!("it is {of} `{list}`, which was declared with `let`"), format!("declare `{list}` with `var`")),
                    Some(Immutable::Param) => {
                        (format!("it is {of} parameter `{list}`, which isn't `var`"), format!("declare it as `var {list}` to change the caller's value"))
                    }
                    _ => return None,
                }
            }
        };
        Some((name, why, help))
    }

    /// The local a place expression is written on (`x` in `x.f[0]`), before following what it
    /// reads from.
    fn written_local(&self, e: &Expr) -> Option<LocalId> {
        match &e.kind {
            ExprKind::Ident(_) => match self.t.res.get(&e.id) {
                Some(Res::Local(l)) => Some(*l),
                _ => None,
            },
            ExprKind::Field { base, .. } | ExprKind::Index { base, .. } => self.written_local(base),
            _ => None,
        }
    }
}
