//! The invariant that keeps stack environments and borrowed interface values safe: a function
//! value with a stack environment (`EnvKind::Stack`), and an interface value that borrows a place
//! (`IfaceData::Borrowed`), appear only as an `Arg::Value` of a `Call`, a `CallIndirect` or a
//! `CallVirtual`, the argument itself. So neither is ever stored, returned or kept in a temporary:
//! each lives exactly as long as the call it is passed to, and a callee that keeps its parameter
//! copies it (`owned` deep-copies a stack environment through the copy function in its header,
//! and an interface value's concrete value through its table's copy entry).
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

/// Whether `v` is an interface value that borrows a place.
pub(crate) fn is_borrowed_iface(v: &Expr) -> bool {
    matches!(v.kind, ExprKind::MakeIface { data: IfaceData::Borrowed(_), .. })
}

/// Checks that every function value with a stack environment and every borrowed interface value
/// in `m` is directly an argument of a `Call`, a `CallIndirect` or a `CallVirtual`. The error names
/// the function where one isn't.
pub fn check_stack_envs(m: &Module) -> Result<(), String> {
    for f in &m.funcs {
        let (mut fns, mut fn_args, mut ifaces, mut iface_args) = (0usize, 0usize, 0usize, 0usize);
        walk(&f.body, &mut |n| match n {
            Node::Expr(e) if is_stack_fn(e) => fns += 1,
            Node::Expr(e) if is_borrowed_iface(e) => ifaces += 1,
            Node::Expr(Expr {
                kind: ExprKind::Call(_, args) | ExprKind::CallIndirect { args, .. } | ExprKind::CallVirtual { args, .. }, ..
            })
            | Node::Stmt(Stmt::Call(_, args) | Stmt::CallIndirect { args, .. } | Stmt::CallVirtual { args, .. }) => {
                fn_args += args.iter().filter(|a| matches!(a, Arg::Value(v) if is_stack_fn(v))).count();
                iface_args += args.iter().filter(|a| matches!(a, Arg::Value(v) if is_borrowed_iface(v))).count();
            }
            _ => {}
        });
        if fns != fn_args {
            return Err(format!("`{}` has a function value with a stack environment that isn't a call's argument", f.name));
        }
        if ifaces != iface_args {
            return Err(format!("`{}` has a borrowed interface value that isn't a call's argument", f.name));
        }
    }
    Ok(())
}
