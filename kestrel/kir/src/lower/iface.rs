//! Interface values in function bodies: making one from a concrete value, and calling a method
//! through one.
//!
//! **Making one.** The checker records each conversion of a struct or enum value to an interface
//! (`Tables::coerce`). A value in `Tables::iface_borrows` is an argument lent to a call: its
//! `MakeIface` is `IfaceData::Borrowed` by the argument's place, so nothing is copied or
//! allocated, and a `var self` method called through it changes the caller's value. Like a
//! closure's stack environment, a borrowed value is only ever a call's `Arg::Value` itself, even
//! when the argument is `var` (never an `Arg::Ref`), and it is never spilled to a temporary: the
//! reordering paths evaluate its place's indexes in turn instead (see `arg_in_turn`). Every other
//! conversion is boxed: the concrete value is made independent (`owned`) and copied into a new
//! block in the current arena.
//!
//! **Calling.** `d.hit(3)` is `CallVirtual`: the method in slot 3 + `slot` of the value's table,
//! given the value's data pointer where the method expects `self`. The arguments are lowered as
//! for a direct call to the interface's abstract method, whose signature they follow.

use kestrel_check::{FnId, IfaceId, Tables, Ty};
use kestrel_syntax::ast;

use super::{kty, FnLowerer};
use crate::ir::*;
use crate::stack_envs::is_stack_fn;

/// The methods that implement interface method `method` (named `name`): the method of that name of
/// every struct and enum that lists its interface, from the checker's tables, whether or not a
/// value of the type is ever made an interface value. A generic declaration and an instance that
/// holds a placeholder have no code of their own (their methods are trapping stubs); a concrete
/// instance's method is its copy, when the collection pass made one (a copy no code reaches
/// isn't made).
pub(crate) fn implementations(t: &Tables, method: FnId, name: &str) -> Vec<FnId> {
    let Some(iface) = t.ifaces.iter().position(|i| i.methods.contains(&method)) else { return Vec::new() };
    let iface = IfaceId(iface as u32);
    let structs = t.structs.iter().map(|s| (&s.ifaces, &s.methods, s.concrete, s.instance.map(|i| i.args)));
    let enums = t.enums.iter().map(|e| (&e.ifaces, &e.methods, e.concrete, e.instance.map(|i| i.args)));
    structs
        .chain(enums)
        .filter(|&(ifaces, _, concrete, _)| concrete && ifaces.contains(&iface))
        .filter_map(|(_, methods, _, args)| {
            let own = *methods.get(name).unwrap_or_else(|| panic!("conformance guarantees a method `{name}`"));
            match args {
                Some(args) => t.generic.fn_copies.get(&(own, args)).copied(),
                None => Some(own),
            }
        })
        .collect()
}

impl FnLowerer<'_> {
    /// `e` converted to the type `to` when that conversion makes an interface value from a struct
    /// or enum: an interface value, or an optional holding a boxed one. `None` otherwise.
    pub(crate) fn iface_coercion(&mut self, e: &ast::Expr, to: Ty) -> Option<Expr> {
        let t = self.t;
        let from = self.ty(e.id);
        if !matches!(from, Ty::Struct(_) | Ty::Enum(_)) {
            return None;
        }
        match to {
            Ty::Iface(i) => Some(self.iface_value(e, i)),
            Ty::Optional(el) => {
                let Ty::Iface(i) = t.elem(el) else { return None };
                // An optional parameter is never lent: the value is boxed.
                let v = self.boxed_iface(e, i);
                let line = v.line;
                Some(Expr { kind: ExprKind::Some(Box::new(v)), ty: kty(to), line })
            }
            _ => None,
        }
    }

    /// The interface value of `e` (a struct or enum value) as interface `i`: borrowed when the
    /// checker lent it, else boxed.
    fn iface_value(&mut self, e: &ast::Expr, i: IfaceId) -> Expr {
        if self.lent(e.id) {
            match self.place(e) {
                Some(p) => return self.borrowed_iface(e, i, p),
                // The checker's `is_kir_place` mirrors `place`; when they disagree, a read-only
                // argument can still be boxed (a `var` one never gets here: see `var_arg`).
                None => debug_assert!(false, "the checker lent an argument that isn't a KIR place"),
            }
        }
        self.boxed_iface(e, i)
    }

    /// The table of `e`'s concrete type as interface `i`.
    fn table(&mut self, e: &ast::Expr, i: IfaceId) -> TableIdx {
        let (t, program) = (self.t, self.program);
        let ty = self.ty(e.id);
        self.helpers.table_for(ty, i, t, program)
    }

    /// An interface value pointing at place `p`, which holds `e`'s value.
    fn borrowed_iface(&mut self, e: &ast::Expr, i: IfaceId, p: Place) -> Expr {
        let table = self.table(e, i);
        Expr { kind: ExprKind::MakeIface { table, data: IfaceData::Borrowed(p) }, ty: KTy::Iface, line: self.line(e.span) }
    }

    /// An interface value holding its own copy of `e`'s value, in the current arena.
    fn boxed_iface(&mut self, e: &ast::Expr, i: IfaceId) -> Expr {
        let table = self.table(e, i);
        let raw = self.expr_raw(e);
        let v = self.owned(raw, false);
        Expr { kind: ExprKind::MakeIface { table, data: IfaceData::Boxed(Box::new(v)) }, ty: KTy::Iface, line: self.line(e.span) }
    }

    /// The argument for a `var` parameter, whose place is `p`: the place itself, or, when the
    /// checker lent the value to a `var` interface parameter, an interface value borrowing it.
    pub(crate) fn var_arg(&mut self, value: &ast::Expr, p: Place) -> Arg {
        if !self.lent(value.id) {
            return Arg::Ref(p);
        }
        let Some(Ty::Iface(i)) = self.coerce(value.id) else {
            unreachable!("a lent argument is converted to an interface")
        };
        Arg::Value(self.borrowed_iface(value, i, p))
    }

    /// The value argument `v`, evaluated in its turn, before later arguments: a temporary holding
    /// it, except for the values that must stay the call's own argument (a stack environment, a
    /// borrowed interface value), whose places' indexes are evaluated now instead.
    pub(crate) fn arg_in_turn(&mut self, mut v: Expr, pre: &mut Vec<Stmt>) -> Expr {
        // A function value with a stack environment stays an argument of the call (see
        // `check_stack_envs`). Making it after its turn changes nothing: the checker's exclusivity
        // rule keeps the call's arguments from changing what it reads.
        if is_stack_fn(&v) {
            return v;
        }
        if let ExprKind::MakeIface { data: IfaceData::Borrowed(p), .. } = &mut v.kind {
            self.stabilize(p, pre);
            return v;
        }
        self.spill_held(v, pre)
    }

    /// The parts of the call `e` of slot `slot` of interface `iface` on its receiver: the
    /// statements to run first, the place of the interface value, the arguments, the signature,
    /// and what the interface method's `@prove` promises with the method itself.
    fn iface_call_parts(&mut self, e: &ast::Expr, iface: IfaceId, slot: u32) -> (Vec<Stmt>, Place, Vec<Arg>, KSig, ContractSet, FuncIdx) {
        let ast::ExprKind::MethodCall { receiver, args, .. } = &e.kind else { unreachable!("an interface call is a method call") };
        let t = self.t;
        let f = t.iface(iface).methods[slot as usize];
        // The abstract method's signature, `self` first, is the one every implementation has.
        let (mut pre, _, mut lowered) = self.call_parts(e, f, args, Some(receiver));
        let recv = match lowered.remove(0) {
            Arg::Ref(p) | Arg::Value(Expr { kind: ExprKind::Load(p), .. }) => p,
            // A value that isn't a place (a call's result, or an `if` or `match` of places, which the
            // checker keeps later arguments from changing; see `check_changed_later`):
            // held in a temporary for the call.
            Arg::Value(v) => self.temp_place(v, &mut pre),
        };
        let info = t.func(f);
        let params = info.params[1..]
            .iter()
            .map(|p| {
                let ty = kty(p.ty);
                (ty, p.is_var || ty.is_aggregate())
            })
            .collect();
        let sig = KSig { params, ret: (info.ret != Ty::Unit).then(|| kty(info.ret)), pure: info.is_pure };
        let keeps = self.helpers.proves.keeps(self.program, info.decl(self.program).span);
        (pre, recv, lowered, sig, keeps, FuncIdx(f.0))
    }

    /// A call through an interface value, as a value.
    pub(crate) fn iface_call_value(&mut self, e: &ast::Expr, iface: IfaceId, slot: u32, ty: KTy, line: u32) -> ExprKind {
        let (pre, recv, args, sig, keeps, method) = self.iface_call_parts(e, iface, slot);
        Self::with_pre(pre, ExprKind::CallVirtual { recv, slot, args, sig, keeps, method }, ty, line)
    }

    /// A call through an interface value whose result is discarded. An owning result is held
    /// until the block ends, as for a direct call.
    pub(crate) fn iface_call_stmt(&mut self, e: &ast::Expr, iface: IfaceId, slot: u32, out: &mut Vec<Stmt>) {
        let (pre, recv, args, sig, keeps, method) = self.iface_call_parts(e, iface, slot);
        out.extend(pre);
        match sig.ret {
            Some(ret) if self.t.is_owning(self.ty(e.id)) => {
                let call = Expr { kind: ExprKind::CallVirtual { recv, slot, args, sig, keeps, method }, ty: ret, line: self.line(e.span) };
                self.keep(call, out);
            }
            _ => out.push(Stmt::CallVirtual { recv, slot, args, sig, keeps, method }),
        }
    }

    /// The interface value `MakeIface { table, data }` made safe to store: a borrowed one becomes
    /// a box holding its own copy of the place's value.
    pub(crate) fn owned_iface(&mut self, table: TableIdx, data: IfaceData, line: u32) -> ExprKind {
        let data = match data {
            IfaceData::Borrowed(p) => {
                let ty = self.helpers.ifaces.concrete(table);
                let v = self.owned(Expr { kind: ExprKind::Load(p), ty, line }, false);
                IfaceData::Boxed(Box::new(v))
            }
            boxed => boxed,
        };
        ExprKind::MakeIface { table, data }
    }
}
