//! The line of a call into a built-in function. A native's error ("draw.circle was called
//! before window.open") should name the line of the program's call, so compiled code stores
//! that line (`Stmt::SetLine`) right before the call.
//!
//! Only calls from the program's own code to a built-in function that isn't `pure` store a line.
//! What built-in code calls inside that function is part of it, and a `pure` function raises no
//! such error. Only the window's thread makes these calls: a `par for` body can't call a
//! function that isn't `pure`.

use kestrel_check::FnId;
use kestrel_syntax::ast;

use super::FnLowerer;
use crate::ir::*;
use crate::visit::{walk_expr, Node};

impl FnLowerer<'_> {
    /// Whether a call to `f` from the function being lowered stores its line.
    fn records_line(&self, f: FnId) -> bool {
        let (t, program) = (self.t, self.program);
        let callee = t.func(f);
        !callee.is_pure
            && t.in_builtin(callee.decl(program).name_span)
            && !t.in_builtin(t.func(self.fn_id).decl(program).name_span)
    }

    /// Whether running `arg` can store a line of its own: it holds a `SetLine`, or calls a
    /// function that isn't `pure` (and so may call into a built-in module somewhere, perhaps
    /// after being inlined here).
    fn may_set_line(&self, arg: &Arg) -> bool {
        let mut found = false;
        let mut look = |node: Node| match node {
            Node::Stmt(Stmt::SetLine(_)) => found = true,
            Node::Expr(Expr { kind: ExprKind::Call(callee, _), .. }) | Node::Stmt(Stmt::Call(callee, _)) => {
                // Past the checked functions are generated helpers, which make no such calls.
                found |= self.t.fns.get(callee.0 as usize).is_some_and(|info| !info.is_pure);
            }
            // A function value may be any function that isn't `pure`.
            Node::Expr(Expr { kind: ExprKind::CallIndirect { .. }, .. }) | Node::Stmt(Stmt::CallIndirect { .. }) => found = true,
            _ => {}
        };
        match arg {
            Arg::Value(v) => walk_expr(v, &mut look),
            Arg::Ref(p) => p.path.iter().filter_map(Proj::at).for_each(|at| walk_expr(at, &mut look)),
        }
        found
    }

    /// Ends the statements `pre` that run before the call `e` to `f` (whose arguments are
    /// `lowered`) with the one that stores the call's line, if `f` is a function that does.
    ///
    /// The arguments run after `pre`. When one could store a line of its own, which would
    /// replace this call's, they are all evaluated in `pre` first, in order, and the call gets
    /// their values.
    pub(crate) fn record_line(&mut self, e: &ast::Expr, f: FnId, lowered: &mut Vec<Arg>, pre: &mut Vec<Stmt>) {
        if !self.records_line(f) {
            return;
        }
        if lowered.iter().any(|a| self.may_set_line(a)) {
            for arg in std::mem::take(lowered) {
                lowered.push(match arg {
                    Arg::Value(v) if matches!(v.kind, ExprKind::Int(_) | ExprKind::Float(_) | ExprKind::Bool(_) | ExprKind::StrLit(_)) => Arg::Value(v),
                    Arg::Value(v) => Arg::Value(self.spill(v, pre)),
                    Arg::Ref(mut p) => {
                        self.stabilize(&mut p, pre);
                        Arg::Ref(p)
                    }
                });
            }
        }
        pre.push(Stmt::SetLine(self.line(e.span)));
    }
}
