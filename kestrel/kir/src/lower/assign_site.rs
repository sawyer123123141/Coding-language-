//! Where a plain `target = value` reuses the target's storage: the value reads an existing place
//! of the same type, the type holds a list, array, map or string and no owning value, the two
//! places can't share memory, and the target's index expressions can't write. Then lowering
//! calls the type's assign helper (see `assign_into`) instead of copying the value and storing
//! the copy. Anything else keeps today's lowering, untouched.
//!
//! Evaluation order is today's: the value's place is found first (its index expressions run, and
//! its bounds are checked), then the target's. Today the value is copied before the target's
//! index runs; reusing reads the value after it, so an index that can write (a call, an `if` that
//! assigns, a `var` argument) could change what is read or where it goes: such a target keeps the
//! copy.

use super::assign_into::reusable;
use super::helpers::owning;
use super::FnLowerer;
use crate::ir::*;
use crate::visit::{walk_expr, Node};

/// Whether places `a` and `b` (element pointers resolved to what they point at) may share
/// memory: the same root, and at each step both have, the same field, or anything else (an
/// index matches any index). It is the rule of the checker's `PathKey::overlaps`
/// (`check/src/body/regions.rs`), on KIR places: only two different fields at the same step keep
/// two paths of one root apart. (Two different variants of one enum, or two fields of one variant,
/// are judged to overlap, which only costs a copy.)
pub(crate) fn overlaps(a: &Place, b: &Place) -> bool {
    a.root == b.root
        && a.path.iter().zip(&b.path).all(|(x, y)| match (x, y) {
            (Proj::Field(i), Proj::Field(j)) => i == j,
            _ => true,
        })
}

/// Whether evaluating `e` can only read: numbers, loads, lengths, tags and arithmetic on them. A
/// call, a `Seq`, an `if`, a random draw, a move or anything that writes a place can't.
fn only_reads(e: &Expr) -> bool {
    let mut ok = true;
    walk_expr(e, &mut |n| match n {
        Node::Expr(x) => {
            ok &= matches!(
                x.kind,
                ExprKind::Int(_)
                    | ExprKind::Float(_)
                    | ExprKind::Bool(_)
                    | ExprKind::Load(_)
                    | ExprKind::Len(_)
                    | ExprKind::Tag(_)
                    | ExprKind::Cast(_)
                    | ExprKind::Unary(..)
                    | ExprKind::Binary(..)
                    | ExprKind::And(..)
                    | ExprKind::Or(..)
            )
        }
        Node::Place(_, written) => ok &= !written,
        Node::Stmt(_) => ok = false,
    });
    ok
}

/// Whether finding place `p` can fail on any of its steps: an index whose bounds aren't known, or
/// a map entry that may be gone. Pass a place with its loop items resolved (`origin`): a loop item
/// is a pointer with no steps, but lowering renames it to its checked element when the loop body
/// may move the collection (`may_move`). Failing reads nested inside index expressions are not
/// looked at; a place with an index expression has that index as a step, which counts already,
/// since a written index is always checked.
fn checks(p: &Place) -> bool {
    p.path.iter().any(|proj| match proj {
        Proj::Index { check, .. } => *check != IndexCheck::Known,
        Proj::MapValue { checked, .. } => *checked,
        _ => false,
    })
}

impl<'a> FnLowerer<'a> {
    /// Whether `target = value`, with `value` lowered as `rhs` and the target's type `ty`, reuses
    /// the target's storage (see the module comment).
    pub(crate) fn reuses(&self, target: &Place, rhs: &Expr, ty: KTy) -> bool {
        let ExprKind::Load(src) = &rhs.kind else { return false };
        if rhs.ty != ty || !reusable(ty, self.t) || owning(ty, self.t) {
            return false;
        }
        if !target.path.iter().filter_map(Proj::at).all(only_reads) {
            return false;
        }
        let (dst_at, src_at) = (self.origin(target), self.origin(src));
        // A capture in a closure's code may share memory with the place it was captured from.
        let captures = [target.root, src.root, dst_at.root, src_at.root];
        if captures.iter().any(|&l| self.captured.as_ref().is_some_and(|c| c.locals.contains(&l))) {
            return false;
        }
        !overlaps(&dst_at, &src_at)
    }

    /// `dst = rhs` (a read of a place, which `reuses` accepted) as a call of the type's assign
    /// helper: the source's place is found first, then the target's, each once.
    pub(crate) fn assign_reusing(&mut self, mut dst: Place, rhs: Expr, out: &mut Vec<Stmt>) {
        let Expr { kind, ty, line } = rhs;
        let ExprKind::Load(mut src) = kind else { unreachable!("`reuses` accepts only a read of a place") };
        self.stabilize(&mut src, out);
        if checks(&self.origin(&src)) && checks(&self.origin(&dst)) {
            // The source's checks run before the target's, as when the value was copied first (a
            // call finds its arguments in order, the target first): a pointer to it is found now.
            // (A loop item's pointer bound here is renamed with everything else if the loop re-finds
            // its items.)
            let r = LocalIdx(self.locals.len() as u32);
            self.locals.push(Local { name: format!("tmp{}", r.0), ty, kind: LocalKind::RefParam });
            self.origins.insert(r, src.clone());
            out.push(Stmt::Bind(r, src));
            src = Place::local(r);
        }
        self.stabilize(&mut dst, out);
        let f = self.helpers.assign_fn(ty, self.t);
        out.push(Stmt::Call(f, vec![Arg::Ref(dst), Arg::Ref(src)], line));
    }
}
