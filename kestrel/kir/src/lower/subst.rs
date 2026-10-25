//! The one way lowering reads the checker's types and resolutions (P19). A copy of a generic is
//! lowered from the generic's declaration, so every type the tables recorded for its expressions,
//! locals, signatures and closures is in the generic's terms (`list[T]`, `Stack[T]`); each read
//! here applies the active substitution: the innermost field default's (`with_subst`), else the
//! copy's. Substitution only looks types up (`subst_known`, G13): the collection pass interned
//! every type a copy reads, and a miss panics naming the copy. With no active substitution
//! (plain code), each accessor is a straight read, so programs without generics lower as before.
//!
//! `kir/tests/subst_discipline.rs` keeps the direct reads (`t.ty(`, `t.res`, `t.coerce`,
//! `iface_borrows`, `t.sig(`, `t.local(`, `t.closures[`) out of every other file of `lower/`.

use kestrel_check::{ClosureInfo, LocalId, LocalInfo, Res, Sig, SigId, Subst, Tables, Ty};
use kestrel_syntax::ast::ExprId;

use super::{kty, FnLowerer};
use crate::ir::KSig;

/// The signature of a call through a value of the checker's function type `sig`, which is
/// already in the terms of the code being lowered (substituted, or from plain code).
pub(super) fn known_ksig(t: &Tables, sig: SigId) -> KSig {
    let sig = t.sig(sig);
    let params = sig
        .params
        .iter()
        .map(|p| {
            let ty = kty(p.ty);
            (ty, p.is_var || ty.is_aggregate())
        })
        .collect();
    KSig { params, ret: (sig.ret != Ty::Unit).then(|| kty(sig.ret)) }
}

/// What a constant expression names, for code outside any function (the constants' build
/// order): constants are never generic, so there is nothing to substitute.
pub(super) fn plain_res(t: &Tables, e: ExprId) -> Option<Res> {
    t.res.get(&e).copied()
}

impl<'a> FnLowerer<'a> {
    /// The substitution the code being lowered is under: the innermost field default's, else the
    /// copy's; `None` in plain code.
    pub(crate) fn active_subst(&self) -> Option<Subst> {
        self.default_substs.last().copied().or(self.subst)
    }

    /// The name a missed type is reported in: the function being lowered.
    fn copy_name(&self) -> &'a str {
        match self.fn_id {
            Some(f) => &self.t.func(f).name,
            None => "a field default",
        }
    }

    /// `ty`, read from the tables, in the terms of the code being lowered.
    pub(crate) fn sub(&self, ty: Ty) -> Ty {
        match self.active_subst() {
            None => ty,
            Some(s) => self.t.subst_known(ty, s, self.copy_name()),
        }
    }

    /// The type of expression `e`.
    pub(crate) fn ty(&self, e: ExprId) -> Ty {
        self.sub(self.t.ty(e))
    }

    /// What the checker knows about local `l`: its name and flags. Its type is `local_ty`.
    pub(crate) fn local(&self, l: LocalId) -> &'a LocalInfo {
        self.t.local(l)
    }

    /// The type of local `l`.
    pub(crate) fn local_ty(&self, l: LocalId) -> Ty {
        self.sub(self.t.local(l).ty)
    }

    /// Signature `s` in the terms of the code being lowered.
    pub(crate) fn sig(&self, s: SigId) -> &'a Sig {
        match self.sub(Ty::Fn(s)) {
            Ty::Fn(s) => self.t.sig(s),
            other => unreachable!("a function type substitutes to a function type, not {other:?}"),
        }
    }

    /// The signature of a call through a value of function type `s`.
    pub(crate) fn ksig(&self, s: SigId) -> KSig {
        match self.sub(Ty::Fn(s)) {
            Ty::Fn(s) => known_ksig(self.t, s),
            other => unreachable!("a function type substitutes to a function type, not {other:?}"),
        }
    }

    /// The conversion the checker recorded for `e`, as a target type. In a copy, a conversion
    /// whose substituted source and target are the same type is no conversion (G14: a `T` passed
    /// as `Damageable` when `T` is `Damageable`).
    pub(crate) fn coerce(&self, e: ExprId) -> Option<Ty> {
        let to = *self.t.coerce.get(&e)?;
        if self.active_subst().is_none() {
            return Some(to);
        }
        let to = self.sub(to);
        (self.ty(e) != to).then_some(to)
    }

    /// Whether `e` is lent to an interface parameter (`Tables::iface_borrows`); dropped with its
    /// conversion when that is dropped (G14).
    pub(crate) fn lent(&self, e: ExprId) -> bool {
        self.t.iface_borrows.contains(&e) && (self.active_subst().is_none() || self.coerce(e).is_some())
    }

    /// What `e` resolves to.
    pub(crate) fn res(&self, e: ExprId) -> Option<Res> {
        self.t.res.get(&e).copied()
    }

    /// What `e` resolves to, when the checker resolves every such expression.
    pub(crate) fn resolved(&self, e: ExprId) -> Res {
        self.res(e).expect("the checker resolves this expression")
    }

    /// Closure literal `e`'s names and flags. Its signature and capture types are in the
    /// literal's terms: read them through `sig` and `sub`.
    pub(crate) fn closure(&self, e: ExprId) -> &'a ClosureInfo {
        &self.t.closures[&e]
    }

    /// Runs `f` under substitution `s` (a field default of an instance, lowered at a
    /// construction).
    // `construct` lowers an instance's missing field defaults under it from Task 6b.
    #[allow(dead_code)]
    pub(crate) fn with_subst<R>(&mut self, s: Subst, f: impl FnOnce(&mut Self) -> R) -> R {
        self.default_substs.push(s);
        let out = f(self);
        self.default_substs.pop();
        out
    }
}
