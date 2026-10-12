//! Keeps plain structs in registers. After inlining, a local of a plain struct type that nothing
//! needs the address of is split into one `Value` local per leaf (`vec2` into `.x` and `.y`), and
//! every struct store becomes one store per leaf (see `split`). A number that had to live in
//! memory only for a `var` argument that inlining removed goes back to being a `Value`.

use std::collections::{HashMap, HashSet};

use crate::ir::*;
use crate::leaves::{leaf_name, leaves};
use crate::split::{Rewriter, Split};
use crate::visit::{walk, Node};
use crate::visit_mut::VisitMut;

pub fn scalarize(m: &mut Module) {
    let Module { funcs, structs, .. } = m;
    for f in funcs.iter_mut().filter(|f| f.native.is_none()) {
        let pinned = pinned(&f.body);
        let mut roots = HashMap::new();
        for i in 0..f.locals.len() {
            let l = LocalIdx(i as u32);
            let local = &f.locals[i];
            if local.kind != LocalKind::Addressed || pinned.contains(&l) {
                continue;
            }
            let Some(all) = leaves(structs, local.ty) else { continue };
            if !local.ty.is_aggregate() {
                f.locals[i].kind = LocalKind::Value;
                continue;
            }
            let (name, ty) = (local.name.clone(), local.ty);
            let locals = all
                .iter()
                .map(|leaf| {
                    f.locals.push(Local { name: leaf_name(structs, &name, ty, leaf), ty: leaf.ty, kind: LocalKind::Value });
                    LocalIdx(f.locals.len() as u32 - 1)
                })
                .collect();
            f.locals[i].kind = LocalKind::Unused;
            roots.insert(l, Split { leaves: all, locals });
        }
        if roots.is_empty() {
            continue;
        }
        let mut rw = Rewriter { structs, locals: &mut f.locals, roots };
        rw.visit_stmts(&mut f.body);
    }
}

/// The locals whose address something needs: passed as `var` (to a call or a `par for`), bound
/// by a loop, or used whole by an operation on memory (moved, dropped, measured, ...).
fn pinned(body: &[Stmt]) -> HashSet<LocalIdx> {
    let mut out = HashSet::new();
    let args = |args: &[Arg], out: &mut HashSet<LocalIdx>| {
        for a in args {
            if let Arg::Ref(p) = a {
                out.insert(p.root);
            }
        }
    };
    walk(body, &mut |n| match n {
        Node::Stmt(Stmt::Call(_, a)) | Node::Stmt(Stmt::ParFor { args: a, .. }) => args(a, &mut out),
        Node::Expr(Expr { kind: ExprKind::Call(_, a), .. }) => args(a, &mut out),
        Node::Stmt(
            Stmt::Bind(_, p)
            | Stmt::ListAdd(p, _)
            | Stmt::Dup(p)
            | Stmt::FreeArena(p)
            | Stmt::DropGpu(p)
            | Stmt::PushArena(Some(p))
            | Stmt::MapSet(p, ..)
            | Stmt::MapUnlock(p, _),
        ) => {
            out.insert(p.root);
        }
        Node::Expr(Expr { kind: ExprKind::Move(p) | ExprKind::Len(p) | ExprKind::Tag(p) | ExprKind::Map(_, p, _), .. }) => {
            out.insert(p.root);
        }
        _ => {}
    });
    out
}
