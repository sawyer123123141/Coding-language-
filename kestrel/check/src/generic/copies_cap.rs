//! The collection pass's names, nesting levels, sizes and caps (G10, P9, P25, P41).
//!
//! A cap reports at the declaration of the generic that keeps repeating on the chain that led
//! there, with its first two copies or instances on that chain: for a copy request, the chain of
//! `made_by`; for a type the walk interns, the chain of default walks when one repeats, else the
//! copy chain of the copy being walked (a type's own declaration may be innocent: a copy of
//! `walk` that only declares a `list[Wrap[T]]` local).
//!
//! Only the two entries shown are ever named, and a type is measured before it is interned, so
//! no oversized name is built.

use std::collections::HashSet;

use kestrel_syntax::Diagnostic;

use crate::generic::copies::{Body, Ctx, Walk, MAX_PARTS};
use crate::generic::{GenericOwner, Subst, TyListId};
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

    /// The copies of `generic` on the `made_by` chain from `from` (included), root first.
    fn copy_chain(&self, generic: FnId, from: Option<FnId>) -> Vec<FnId> {
        let mut chain = Vec::new();
        let mut by = from;
        while let Some(c) = by {
            let Some(copy) = self.t.func(c).copy else { break };
            if copy.generic == generic {
                chain.push(c);
            }
            by = copy.made_by;
        }
        chain.reverse();
        chain
    }

    /// The cap's error for the copy of `f` for `args`, requested under `ctx`.
    pub(super) fn copy_cap(&self, f: FnId, args: TyListId, ctx: Ctx, stopped: &str) -> Diagnostic {
        let mut shown: Vec<String> = self.copy_chain(f, ctx.made_by).iter().take(2).map(|&c| self.t.func(c).name.clone()).collect();
        if shown.len() < 2 {
            shown.push(self.copy_name(f, args));
        }
        let generic = self.t.func(f);
        Diagnostic::error(generic.span, cap_message(&generic.name, &shown, stopped))
    }

    /// The cap's error for a type the walk interns (or would intern) under `ctx`. `failing` is the
    /// generic whose instance passed the cap, and the instance when it is interned (and small).
    pub(super) fn walk_cap(&self, ctx: Ctx, failing: (GenericOwner, Option<Ty>), stopped: &str) -> Diagnostic {
        // The nearest default walk on the chain, and its declaration's instances along it.
        let mut units = Vec::new();
        let mut at = ctx.unit;
        while let Some(u) = at {
            units.push(u);
            at = self.units[u].parent;
        }
        let def = units.iter().find_map(|&u| match self.units[u].body {
            Body::Default(inst, _) => self.t.strukt(inst).instance.map(|i| i.def),
            Body::Copy(_) => None,
        });
        let mut defaults: Vec<Ty> = units
            .iter()
            .rev()
            .filter_map(|&u| match self.units[u].body {
                Body::Default(inst, _) if self.t.strukt(inst).instance.map(|i| i.def) == def => Some(Ty::Struct(inst)),
                _ => None,
            })
            .collect();
        // An instance whose defaults are walked for several fields is one step.
        let mut seen = HashSet::new();
        defaults.retain(|&ty| seen.insert(ty));
        let copies = match ctx.made_by.and_then(|c| self.t.func(c).copy) {
            Some(copy) => self.copy_chain(copy.generic, ctx.made_by),
            None => Vec::new(),
        };
        if let Some(def) = def.filter(|_| defaults.len() >= 2 || copies.len() < 2) {
            let shown: Vec<String> = defaults.iter().take(2).map(|&ty| self.t.ty_name(ty)).collect();
            let owner = GenericOwner::Struct(def);
            return Diagnostic::error(self.t.owner_span(owner), cap_message(self.t.owner_name(owner), &shown, stopped));
        }
        if let Some(&last) = copies.last() {
            let generic = self.t.func(last).copy.expect("a copy").generic;
            let shown: Vec<String> = copies.iter().take(2).map(|&c| self.t.func(c).name.clone()).collect();
            let info = self.t.func(generic);
            return Diagnostic::error(info.span, cap_message(&info.name, &shown, stopped));
        }
        // Nothing on the chain (a substitution is always some unit's, so this isn't expected).
        let (owner, ty) = failing;
        let shown = vec![ty.map_or_else(|| self.t.owner_name(owner).to_string(), |ty| self.t.ty_name(ty))];
        Diagnostic::error(self.t.owner_span(owner), cap_message(self.t.owner_name(owner), &shown, stopped))
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

    /// How many parts a concrete `ty` has: 1 plus its parts' (an instance's type arguments, an
    /// element, a map's key and value, a function's parameters and result). Saturating.
    fn size(&mut self, ty: Ty) -> u32 {
        if let Some(&n) = self.size.get(&ty) {
            return n;
        }
        let parts = match ty {
            Ty::Struct(s) => self.t.strukt(s).instance.map(|i| self.t.ty_list(i.args).to_vec()).unwrap_or_default(),
            Ty::Enum(e) | Ty::Variant(e, _) => self.t.enm(e).instance.map(|i| self.t.ty_list(i.args).to_vec()).unwrap_or_default(),
            Ty::Array(e) | Ty::List(e) | Ty::Optional(e) => vec![self.t.elem(e)],
            Ty::Map(k, v) => vec![self.t.elem(k), self.t.elem(v)],
            Ty::Fn(sig) => {
                let sig = self.t.sig(sig);
                sig.params.iter().map(|p| p.ty).chain([sig.ret]).collect()
            }
            _ => Vec::new(),
        };
        let n = parts.into_iter().fold(1u32, |n, p| n.saturating_add(self.size(p)));
        self.size.insert(ty, n);
        n
    }

    /// The size `ty` will have under `s`, worked out without interning anything; `Err` with the
    /// generic whose instance's type list would pass `MAX_PARTS` (P41).
    pub(super) fn size_under(&mut self, ty: Ty, s: Subst) -> Result<u32, GenericOwner> {
        if self.t.is_concrete(ty) {
            return Ok(self.size(ty));
        }
        let (parts, owner) = match ty {
            Ty::Param(p) => {
                let index = self.t.generic.param(p).index as usize;
                let arg = self.t.ty_list(s.args).get(index).copied().unwrap_or(Ty::Error);
                return Ok(self.size(arg));
            }
            Ty::Struct(id) => {
                let (def, args) = self.t.struct_parts(id);
                (args, Some(GenericOwner::Struct(def)))
            }
            Ty::Enum(id) | Ty::Variant(id, _) => {
                let (def, args) = self.t.enum_parts(id);
                (args, Some(GenericOwner::Enum(def)))
            }
            Ty::Array(e) | Ty::List(e) | Ty::Optional(e) => (vec![self.t.elem(e)], None),
            Ty::Map(k, v) => (vec![self.t.elem(k), self.t.elem(v)], None),
            Ty::Fn(sig) => {
                let sig = self.t.sig(sig);
                (sig.params.iter().map(|p| p.ty).chain([sig.ret]).collect(), None)
            }
            _ => return Ok(1),
        };
        let mut inner = 0u32;
        for p in parts {
            inner = inner.saturating_add(self.size_under(p, s)?);
        }
        match owner {
            Some(owner) if inner > MAX_PARTS => Err(owner),
            _ => Ok(inner.saturating_add(1)),
        }
    }

    /// As `size_under`, for a type list that will be interned as it is (`type_args`).
    pub(super) fn list_size_under(&mut self, l: TyListId, s: Subst) -> Result<u32, GenericOwner> {
        let tys = self.t.ty_list(l).to_vec();
        let mut total = 0u32;
        for ty in tys {
            total = total.saturating_add(self.size_under(ty, s)?);
        }
        if total > MAX_PARTS {
            // The list belongs to the owner being substituted (its callee's arguments).
            return Err(s.owner);
        }
        Ok(total)
    }
}

/// "making copies of `walk` doesn't end: `walk[Box[int]]` needs `walk[Box[Box[int]]]`, and so on
/// (stopped after 32 levels)", from the chain's first two (the first twice when it has one).
fn cap_message(generic: &str, shown: &[String], stopped: &str) -> String {
    let first = &shown[0];
    let second = shown.get(1).unwrap_or(first);
    format!("making copies of `{generic}` doesn't end: `{first}` needs `{second}`, and so on (stopped after {stopped})")
}
