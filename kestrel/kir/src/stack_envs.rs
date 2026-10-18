//! The invariant that keeps stack environments safe: a function value with a stack environment
//! (`EnvKind::Stack`) appears only as an `Arg::Value` of a `Call` or a `CallIndirect`, the
//! argument itself. So it is never stored, returned or kept in a temporary: it lives exactly as
//! long as the call it is passed to, and a callee that keeps its parameter copies it (`owned`
//! deep-copies it, through the copy function in its header).
//!
//! `lower` checks this on its own output in debug builds. The passes after it may move an
//! argument into a local of the inlined body (`inline_small`), which lives no longer than the call
//! would have.

use crate::ir::*;
use crate::visit::{walk, Node};

/// Whether `v` is a function value with a stack environment.
pub(crate) fn is_stack_fn(v: &Expr) -> bool {
    matches!(v.kind, ExprKind::MakeFn { env: EnvKind::Stack(_), .. })
}

/// Checks that every function value with a stack environment in `m` is directly an argument of a
/// `Call` or a `CallIndirect`. The error names the function where one isn't.
pub fn check_stack_envs(m: &Module) -> Result<(), String> {
    for f in &m.funcs {
        let (mut all, mut as_args) = (0usize, 0usize);
        walk(&f.body, &mut |n| match n {
            Node::Expr(e) if is_stack_fn(e) => all += 1,
            Node::Expr(Expr { kind: ExprKind::Call(_, args) | ExprKind::CallIndirect { args, .. }, .. })
            | Node::Stmt(Stmt::Call(_, args) | Stmt::CallIndirect { args, .. }) => {
                as_args += args.iter().filter(|a| matches!(a, Arg::Value(v) if is_stack_fn(v))).count();
            }
            _ => {}
        });
        if all != as_args {
            return Err(format!("`{}` has a function value with a stack environment that isn't a call's argument", f.name));
        }
    }
    Ok(())
}
