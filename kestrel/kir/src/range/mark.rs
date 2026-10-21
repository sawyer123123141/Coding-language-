//! Marking what the analysis found: every proven arithmetic node is wrapped in `Proven`, every
//! proven index is marked, and every `@assume` statement gets its `AssumeCheck`.
//!
//! Under `Arith::Checked` every assumption is `Check`, set actively, so a module marked before
//! under `--release` can't keep a `Trust` (the addendum's "`@assume`", "Checked builds"). Under
//! `Arith::Wrapping` an assumption statement is `Trust` exactly when the analysis recorded a use
//! of it whose real run proves no bounds check its plain run leaves (soundness rule 6); every
//! other one, a function the analysis gave up on included, is `Check`. Trust is keyed by the
//! statement's address, never by its conditions' ids: two inlined copies of one assumption in one
//! function are decided on their own (C3).

use std::collections::{HashMap, HashSet};

use super::{expr_key, index_key, Analysis, Arith, Key};
use crate::ir::*;
use crate::visit_mut::{walk_expr, walk_place, walk_stmt, VisitMut};

/// Wraps every proven arithmetic node in `Proven`, marks every proven index, and sets each
/// assumption's check: `Trust` for the statements in `trusted` (by address), `Check` for the rest.
pub(super) struct Marker<'a> {
    pub proven: &'a HashMap<Key, ()>,
    pub trusted: &'a HashSet<usize>,
}

impl VisitMut for Marker<'_> {
    fn visit_stmt(&mut self, s: &mut Stmt) {
        let at = s as *const Stmt as usize;
        if let Stmt::Assume { check, .. } = s {
            *check = if self.trusted.contains(&at) { AssumeCheck::Trust } else { AssumeCheck::Check };
        }
        walk_stmt(self, s);
    }

    fn visit_expr(&mut self, e: &mut Expr) {
        let key = expr_key(e);
        walk_expr(self, e);
        if self.proven.contains_key(&key) {
            let (ty, line) = (e.ty, e.line);
            let inner = std::mem::replace(e, Expr { kind: ExprKind::Bool(false), ty, line });
            *e = Expr { kind: ExprKind::Proven(Box::new(inner)), ty, line };
        }
    }

    fn visit_place(&mut self, p: &mut Place) {
        for proj in &mut p.path {
            if self.proven.contains_key(&index_key(proj)) {
                if let Proj::Index { check, .. } = proj {
                    *check = IndexCheck::Proven;
                }
            }
        }
        walk_place(self, p);
    }
}

/// The assumption statements of function `func` that `arith` lets go unevaluated, by address:
/// none in a checked build; under `--release` those with a use that rests no bounds check on it.
pub(super) fn trusted(a: &Analysis, func: usize, arith: Arith) -> HashSet<usize> {
    if arith.checked() {
        return HashSet::new();
    }
    a.assumptions.iter().filter(|u| u.func.0 as usize == func && !u.bounds).map(|u| u.at).collect()
}

/// Sets every assumption statement's check in `m` from `a` (an analysis of `m` as it is now, so
/// the statements' addresses still match) for a build in `arith`, and changes nothing else.
pub fn mark_assumptions(m: &mut Module, a: &Analysis, arith: Arith) {
    let none = HashMap::new();
    for i in 0..m.funcs.len() {
        if m.funcs[i].native.is_some() {
            continue;
        }
        let trusted = trusted(a, i, arith);
        Marker { proven: &none, trusted: &trusted }.visit_stmts(&mut m.funcs[i].body);
    }
}
