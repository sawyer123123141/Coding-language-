use std::collections::HashMap;

use kestrel_check::{EnumId, FnId, Res, StructId};
use kestrel_syntax::ast;

use super::FnLowerer;
use crate::ir::*;

impl<'a> FnLowerer<'a> {
    /// The place an expression names (`x`, `x.f.g`), if it is one.
    pub(crate) fn place(&self, e: &ast::Expr) -> Option<Place> {
        match &e.kind {
            ast::ExprKind::Ident(_) => match self.t.res.get(&e.id) {
                Some(Res::Local(l)) => Some(Place::local(self.local_map[l])),
                _ => None,
            },
            ast::ExprKind::Field { base, .. } => {
                // `Enum.Variant` parses as a field access but isn't a place.
                let field = *self.t.field_index.get(&e.id)?;
                if let Some(Res::Local(l)) = self.t.res.get(&base.id) {
                    if let Some((p, variant)) = self.bindings.get(l) {
                        let mut p = p.clone();
                        p.path.push(Proj::Variant(*variant, field));
                        return Some(p);
                    }
                }
                let mut p = self.place(base)?;
                p.path.push(Proj::Field(field));
                Some(p)
            }
            _ => None,
        }
    }

    /// Stores `v` in a fresh temporary and returns a load of it.
    fn spill(&mut self, v: Expr, pre: &mut Vec<Stmt>) -> Expr {
        let (ty, line) = (v.ty, v.line);
        let tmp = self.temp(ty);
        pre.push(Stmt::Let(tmp, v));
        Expr { kind: ExprKind::Load(Place::local(tmp)), ty, line }
    }

    /// A call's arguments in parameter order, plus statements to run first.
    /// Arguments are evaluated in source order: when labels reorder them,
    /// value arguments (and a read-only receiver) go through temporaries.
    pub(crate) fn call_parts(
        &mut self,
        e: &ast::Expr,
        f: FnId,
        args: &[ast::Arg],
        receiver: Option<&ast::Expr>,
    ) -> (Vec<Stmt>, FuncIdx, Vec<Arg>) {
        let t = self.t;
        let params = &t.func(f).params;
        let skip = usize::from(receiver.is_some());
        let order = &t.call_args[&e.id];
        let in_order = order.iter().enumerate().all(|(i, &ai)| i == ai);
        let mut pre = Vec::new();
        let mut lowered = Vec::with_capacity(params.len());
        if let Some(r) = receiver {
            if params[0].is_var {
                lowered.push(Arg::Ref(self.place(r).expect("checker requires a place for a `var self` call")));
            } else {
                let v = self.expr(r);
                let v = if in_order { v } else { self.spill(v, &mut pre) };
                lowered.push(Arg::Value(v));
            }
        }
        let mut temps: HashMap<usize, Expr> = HashMap::new();
        if !in_order {
            for (ai, a) in args.iter().enumerate() {
                let pi = order.iter().position(|&x| x == ai).expect("every argument is bound");
                if !params[skip + pi].is_var {
                    let v = self.expr(&a.value);
                    temps.insert(ai, self.spill(v, &mut pre));
                }
            }
        }
        for (pi, &ai) in order.iter().enumerate() {
            let value = &args[ai].value;
            lowered.push(if params[skip + pi].is_var {
                Arg::Ref(self.place(value).expect("checker requires a place for a `var` argument"))
            } else if let Some(v) = temps.remove(&ai) {
                Arg::Value(v)
            } else {
                Arg::Value(self.expr(value))
            });
        }
        (pre, FuncIdx(f.0), lowered)
    }

    /// `Point(...)`: every field in declaration order, defaults filled in,
    /// arguments evaluated in source order.
    pub(crate) fn construct(&mut self, e: &ast::Expr, sid: StructId, args: &[ast::Arg]) -> (Vec<Stmt>, ExprKind) {
        let (t, program) = (self.t, self.program);
        let slots = &t.ctor_args[&e.id];
        let ast::Item::Struct(decl) = &program.items[t.strukt(sid).item] else { unreachable!("StructInfo points at a struct") };
        let bound: Vec<usize> = slots.iter().flatten().copied().collect();
        let in_order = bound.windows(2).all(|w| w[0] < w[1]);
        let mut pre = Vec::new();
        let mut temps: HashMap<usize, Expr> = HashMap::new();
        if !in_order {
            for (ai, a) in args.iter().enumerate() {
                let v = self.expr(&a.value);
                temps.insert(ai, self.spill(v, &mut pre));
            }
        }
        let mut fields = Vec::with_capacity(slots.len());
        for (fi, slot) in slots.iter().enumerate() {
            fields.push(match slot {
                Some(ai) => match temps.remove(ai) {
                    Some(v) => v,
                    None => self.expr(&args[*ai].value),
                },
                None => self.expr(decl.fields[fi].default.as_ref().expect("checker requires a default for a missing field")),
            });
        }
        (pre, ExprKind::StructLit(StructIdx(sid.0), fields))
    }

    /// `Block.Stone(...)`: the variant's fields in declaration order,
    /// arguments evaluated in source order.
    pub(crate) fn variant_construct(&mut self, e: &ast::Expr, id: EnumId, variant: u32, args: &[ast::Arg]) -> (Vec<Stmt>, ExprKind) {
        let t = self.t;
        let slots = &t.ctor_args[&e.id];
        let bound: Vec<usize> = slots.iter().flatten().copied().collect();
        let in_order = bound.windows(2).all(|w| w[0] < w[1]);
        let mut pre = Vec::new();
        let mut values: Vec<Option<Expr>> = (0..args.len()).map(|_| None).collect();
        if !in_order {
            for (ai, a) in args.iter().enumerate() {
                let v = self.expr(&a.value);
                values[ai] = Some(self.spill(v, &mut pre));
            }
        }
        let mut fields = Vec::with_capacity(slots.len());
        for slot in slots {
            let ai = slot.expect("checker binds every variant field");
            fields.push(match values[ai].take() {
                Some(v) => v,
                None => self.expr(&args[ai].value),
            });
        }
        (pre, ExprKind::VariantLit(EnumIdx(id.0), variant, fields))
    }

    /// Wraps `kind` so `pre` runs first, if there is anything to run.
    pub(crate) fn with_pre(pre: Vec<Stmt>, kind: ExprKind, ty: KTy, line: u32) -> ExprKind {
        if pre.is_empty() {
            kind
        } else {
            ExprKind::Seq(pre, Box::new(Expr { kind, ty, line }))
        }
    }
}
