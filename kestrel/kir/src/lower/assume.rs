//! `@assume`: an assumption covers the rest of its block, so lowering puts the statements after it,
//! up to the block's end, inside `Stmt::Assume`. Assumptions with no statement between them are one
//! `Stmt::Assume` with several conditions.
//!
//! A block that gives a value computes it inside the innermost assumption's body, into a
//! temporary (moved out when it has heap memory), so the block's value is that temporary and
//! nothing after the `Assume` but the block's cleanup runs before it is used. A function's body
//! does the same with its final `return e`, so the function still ends in `Stmt::Return` and the
//! inliner still takes it. A value that only reads a whole local, in a block with no cleanup, is
//! given after the `Assume` as written instead (`as_written`).

use kestrel_syntax::ast;

use super::helpers::has_heap;
use super::FnLowerer;
use crate::ir::*;

/// What a block's statements end in.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Ending {
    /// Statements only: a block that gives no value.
    Stmts,
    /// A block used as a value (an `if` or `match` arm): its last expression is the value.
    Value,
    /// The body of a function or closure that gives a value: as `Value`, and a final `return e`
    /// inside an assumption gives the value too, so the `return` comes after the assumption.
    Body,
}

impl FnLowerer<'_> {
    /// Lowers a block's statements into `out`. For `Value` and `Body`, gives the block's value
    /// (`None` when it can't finish normally) and whether that value is already a temporary made
    /// inside an assumption (then it needs no further care before the block's cleanup).
    pub(crate) fn block_rest(&mut self, stmts: &[ast::Stmt], ending: Ending, out: &mut Vec<Stmt>) -> (Option<Expr>, bool) {
        self.rest(stmts, ending, false, out)
    }

    fn rest(&mut self, stmts: &[ast::Stmt], ending: Ending, inside: bool, out: &mut Vec<Stmt>) -> (Option<Expr>, bool) {
        let gives = matches!(ending, Ending::Value | Ending::Body);
        for (i, s) in stmts.iter().enumerate() {
            if matches!(s, ast::Stmt::Assume { .. }) {
                return self.assumption(&stmts[i..], ending, out);
            }
            if i + 1 < stmts.len() {
                self.stmt(s, out);
                continue;
            }
            match s {
                ast::Stmt::Expr(e) if gives && !self.leaves_early(e) => {
                    let v = self.expr(e);
                    if !inside || self.as_written(&v) {
                        return (Some(v), false);
                    }
                    // As `block_value` and `lower_body` treat it: a function's body with no cleanup
                    // returns it with `owned(v, true)`; otherwise it is computed before the cleanup.
                    let moving = ending == Ending::Body && !self.scope_has_cleanup();
                    return (Some(self.settle(v, moving, out)), true);
                }
                // As `return_stmt` treats it.
                ast::Stmt::Return { value: Some(e), .. } if inside && ending == Ending::Body => {
                    let v = self.expr(e);
                    if self.as_written(&v) {
                        return (Some(v), false);
                    }
                    return (Some(self.settle(v, true, out)), true);
                }
                _ => self.stmt(s, out),
            }
        }
        (None, false)
    }

    /// The assumptions at the start of `stmts` (one `Stmt::Assume`), with the rest of the block
    /// as its body.
    fn assumption(&mut self, stmts: &[ast::Stmt], ending: Ending, out: &mut Vec<Stmt>) -> (Option<Expr>, bool) {
        let n = stmts.iter().take_while(|s| matches!(s, ast::Stmt::Assume { .. })).count();
        let mut conds = Vec::with_capacity(n);
        for s in &stmts[..n] {
            let ast::Stmt::Assume { cond, text, span } = s else { unreachable!("counted above") };
            let id = u32::try_from(self.helpers.assumptions.len()).expect("fewer than 2^32 assumptions");
            let line = self.line(*span);
            self.helpers.assumptions.push(AssumeInfo { text: text.clone(), span: *span, line });
            conds.push((id, self.expr(cond)));
        }
        let mut body = Vec::new();
        let (value, settled) = self.rest(&stmts[n..], ending, true, &mut body);
        out.push(Stmt::Assume { conds, check: AssumeCheck::Check, body });
        (value, settled)
    }

    /// Whether the block's value `v` can be given after the assumption as it is, unsettled, so the
    /// block ends exactly as it would without the assumption: `v` reads a whole local that isn't a
    /// parameter passed by pointer (nothing runs between the assumption's end and the read, so the
    /// local still holds it) and the block has no cleanup. A temporary would hide a returned list's
    /// length from range analysis, which tracks it only for locals it indexes or measures. With a
    /// cleanup the value is settled before it, in a temporary, either way; a parameter's list is
    /// copied into one, either way, and the copy is made inside the assumption.
    fn as_written(&self, v: &Expr) -> bool {
        let (ExprKind::Load(p) | ExprKind::Move(p)) = &v.kind else { return false };
        v.is_local_read() && self.locals[p.root.0 as usize].kind != LocalKind::RefParam && !self.scope_has_cleanup()
    }

    /// A block's value `v`, computed into a temporary at the end of an assumption's body, the way
    /// `block_value` computes one before a cleanup: `owned(v, moving)` (with `moving`, a local is
    /// moved and a parameter copied, as a `return` does), then a temporary that a value with heap
    /// memory is moved out of.
    fn settle(&mut self, v: Expr, moving: bool, out: &mut Vec<Stmt>) -> Expr {
        let v = self.owned(v, moving);
        let spilled = self.spill(v, out);
        match spilled.kind {
            ExprKind::Load(place) if has_heap(spilled.ty, self.t) => Expr { kind: ExprKind::Move(place), ..spilled },
            _ => spilled,
        }
    }
}
