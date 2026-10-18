//! `par for`: the loop becomes a function of its own over a range of
//! indexes (`__kestrel_par_N`), which the runtime runs on many threads
//! through `Stmt::ParFor`. Variables from outside the body are passed to it
//! by pointer; the body only reads them (the checker makes sure).

use std::collections::BTreeSet;

use kestrel_syntax::ast;

use super::FnLowerer;
use crate::ir::*;
use crate::rename::{rename_stmts, Subst};
use crate::visit::{walk, Node};

/// The locals `stmts` define (`let`, a loop variable, a bound element)
/// and the ones they use at all.
fn locals_in(stmts: &[Stmt]) -> (BTreeSet<LocalIdx>, BTreeSet<LocalIdx>) {
    let (mut defined, mut used) = (BTreeSet::new(), BTreeSet::new());
    walk(stmts, &mut |n| match n {
        Node::Stmt(Stmt::Let(l, _) | Stmt::Bind(l, _) | Stmt::BindEnv { local: l, .. } | Stmt::ForRange { var: l, .. }) => {
            defined.insert(*l);
            used.insert(*l);
        }
        Node::Place(p, _) => {
            used.insert(p.root);
        }
        _ => {}
    });
    (defined, used)
}

impl<'a> FnLowerer<'a> {
    /// `par for i, v in xs { body }`: lowers the loop over `start..end`
    /// (parameters of the new function), moves it into a function of its
    /// own, and leaves a `ParFor` over `0..xs.len` in its place.
    pub(crate) fn par_for_in(
        &mut self,
        index: Option<&ast::Binding>,
        var: &ast::Binding,
        iter: &ast::Expr,
        body: &ast::Block,
        out: &mut Vec<Stmt>,
    ) {
        let line = self.line(iter.span);
        let list = self.place_or_temp(iter, out, true);
        let (start, end) = (self.temp(KTy::I64), self.temp(KTy::I64));
        let bound = |l: LocalIdx| {
            let load = Expr { kind: ExprKind::Load(Place::local(l)), ty: KTy::I64, line };
            Expr { kind: ExprKind::Cast(Box::new(load)), ty: KTy::I32, line }
        };
        let mut stmts = vec![self.list_loop(index, var, list.clone(), body, bound(start), bound(end))];

        // What the body defines is its own; anything else it uses comes from outside.
        let (defined, used) = locals_in(&stmts);
        let captured: Vec<LocalIdx> =
            used.iter().copied().filter(|l| !defined.contains(l) && *l != start && *l != end).collect();
        let mut locals = Vec::new();
        let mut subst = vec![Subst::Local(LocalIdx(0)); self.locals.len()];
        for &c in &captured {
            // Passed by pointer, so a value kept in a register needs memory.
            if self.locals[c.0 as usize].kind == LocalKind::Value {
                self.locals[c.0 as usize].kind = LocalKind::Addressed;
            }
            let Local { name, ty, .. } = self.locals[c.0 as usize].clone();
            subst[c.0 as usize] = Subst::Place(Place::local(LocalIdx(locals.len() as u32)));
            locals.push(Local { name, ty, kind: LocalKind::RefParam });
        }
        for (l, name) in [(start, "start"), (end, "end")] {
            subst[l.0 as usize] = Subst::Local(LocalIdx(locals.len() as u32));
            locals.push(Local { name: name.into(), ty: KTy::I64, kind: LocalKind::Value });
        }
        let params = (0..locals.len() as u32).map(LocalIdx).collect();
        for &d in &defined {
            subst[d.0 as usize] = Subst::Local(LocalIdx(locals.len() as u32));
            locals.push(self.locals[d.0 as usize].clone());
        }
        rename_stmts(&mut stmts, &subst);

        let func = self.helpers.add_par_body(Func {
            name: String::new(),
            params,
            ret: None,
            locals,
            body: stmts,
            is_pure: true,
            io: false,
            mutates_params: true,
            exported: false,
            native: None,
        });
        let args = captured.into_iter().map(|c| Arg::Ref(Place::local(c))).collect();
        let count = Expr { kind: ExprKind::Len(list), ty: KTy::I64, line };
        out.push(Stmt::ParFor { func, args, count });
    }
}
