//! The collection pass's names, nesting levels and caps (G10, P9, P25).
//!
//! The cap reports at the declaration of the generic whose copy (or instance) passed it, with that
//! generic's first two copies on the chain that led there: for a copy, the chain of `made_by`;
//! for an instance, the chain of default walks.

use std::collections::HashSet;

use kestrel_syntax::Diagnostic;

use crate::generic::copies::{Body, Ctx, Walk};
use crate::generic::{GenericOwner, TyListId};
use crate::tables::FnId;
use crate::ty::Ty;

impl Walk<'_> {
    /// A copy's name: `best[Enemy]`, `Stack[int].push`, `geom.Stack[geom.Item].push`.
    pub(super) fn copy_name(&self, f: FnId, args: TyListId) -> String {
        let info = self.t.func(f);
        let tys = self.t.ty_list(args);
        let method = info.name.rsplit('.').next().unwrap_or_default();
        match info.generic {
            Some(GenericOwner::Struct(def)) => format!("{}.{method}", self.t.instance_name(&self.t.strukt(def).name, tys)),
            Some(GenericOwner::Enum(def)) => format!("{}.{method}", self.t.instance_name(&self.t.enm(def).name, tys)),
            _ => self.t.instance_name(&info.name, tys),
        }
    }

    /// The cap's error for the copy of `f` for `args`, requested under `ctx`.
    pub(super) fn copy_cap(&self, f: FnId, args: TyListId, ctx: Ctx, stopped: &str) -> Diagnostic {
        let mut chain = vec![self.copy_name(f, args)];
        let mut by = ctx.made_by;
        while let Some(c) = by {
            let info = self.t.func(c);
            let Some(copy) = info.copy else { break };
            if copy.generic == f {
                chain.push(info.name.clone());
            }
            by = copy.made_by;
        }
        chain.reverse();
        let generic = self.t.func(f);
        Diagnostic::error(generic.span, cap_message(&generic.name, &chain, stopped))
    }

    /// The cap's error for the instance `failing` of `def`, interned (or walked) under `ctx`.
    pub(super) fn type_cap(&self, def: GenericOwner, failing: Ty, ctx: Ctx, stopped: &str) -> Diagnostic {
        let mut chain = vec![failing];
        let mut at = ctx.unit;
        while let Some(u) = at {
            if let Body::Default(inst, _) = self.units[u].body {
                if self.t.strukt(inst).instance.map(|i| GenericOwner::Struct(i.def)) == Some(def) {
                    chain.push(Ty::Struct(inst));
                }
            }
            at = self.units[u].parent;
        }
        chain.reverse();
        // An instance whose defaults are walked for several fields is one step.
        let mut seen = HashSet::new();
        chain.retain(|&ty| seen.insert(ty));
        let names: Vec<String> = chain.iter().map(|&ty| self.t.ty_name(ty)).collect();
        Diagnostic::error(self.t.owner_span(def), cap_message(self.t.owner_name(def), &names, stopped))
    }

    /// How many levels `ty` nests: `int` is 0, `list[int]` and `Box[int]` are 1.
    pub(super) fn nest(&mut self, ty: Ty) -> u32 {
        if let Some(&n) = self.nest.get(&ty) {
            return n;
        }
        let n = match ty {
            Ty::Struct(s) => match self.t.strukt(s).instance {
                Some(i) => 1 + self.list_nest(i.args),
                None => 0,
            },
            Ty::Enum(e) | Ty::Variant(e, _) => match self.t.enm(e).instance {
                Some(i) => 1 + self.list_nest(i.args),
                None => 0,
            },
            Ty::Array(e) | Ty::List(e) | Ty::Optional(e) => 1 + self.nest(self.t.elem(e)),
            Ty::Map(k, v) => {
                let k = self.nest(self.t.elem(k));
                1 + k.max(self.nest(self.t.elem(v)))
            }
            Ty::Fn(sig) => {
                let sig = self.t.sig(sig).clone();
                let params = sig.params.iter().map(|p| self.nest(p.ty)).max().unwrap_or(0);
                1 + params.max(self.nest(sig.ret))
            }
            _ => 0,
        };
        self.nest.insert(ty, n);
        n
    }

    /// The most levels any type of a list nests.
    pub(super) fn list_nest(&mut self, l: TyListId) -> u32 {
        let tys = self.t.ty_list(l).to_vec();
        tys.into_iter().map(|a| self.nest(a)).max().unwrap_or(0)
    }
}

/// "making copies of `walk` doesn't end: `walk[Box[int]]` needs `walk[Box[Box[int]]]`, and so on
/// (stopped after 32 levels)", from the chain's first two (the first twice when it has one).
fn cap_message(generic: &str, chain: &[String], stopped: &str) -> String {
    let first = &chain[0];
    let second = chain.get(1).unwrap_or(first);
    format!("making copies of `{generic}` doesn't end: `{first}` needs `{second}`, and so on (stopped after {stopped})")
}
