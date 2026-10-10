use std::collections::HashMap;

use kestrel_check::{EnumId, FnId, Res, StructId};
use kestrel_syntax::ast;

use super::FnLowerer;
use crate::ir::*;

impl<'a> FnLowerer<'a> {
    /// The place an expression names (`x`, `x.f.g`, `xs[i]`), if it is one.
    pub(crate) fn place(&mut self, e: &ast::Expr) -> Option<Place> {
        match &e.kind {
            ast::ExprKind::Ident(_) => match self.t.res.get(&e.id) {
                Some(Res::Local(l)) => Some(match self.elems.get(l) {
                    Some(p) => p.clone(),
                    None => Place::local(self.local_map[l]),
                }),
                _ => None,
            },
            // `m[k]` is a lookup, not a place.
            ast::ExprKind::Index { base, .. } if matches!(self.t.ty(base.id), kestrel_check::Ty::Map(..)) => None,
            ast::ExprKind::Index { base, index } => {
                let mut p = self.place(base)?;
                p.path.push(Proj::Index { at: self.expr(index), checked: true });
                Some(p)
            }
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
    pub(crate) fn spill(&mut self, v: Expr, pre: &mut Vec<Stmt>) -> Expr {
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
                let mut p = self.place(r).expect("checker requires a place for a `var self` call");
                if !in_order {
                    self.stabilize(&mut p, &mut pre);
                }
                lowered.push(Arg::Ref(p));
            } else {
                let v = self.expr(r);
                let v = if in_order { v } else { self.spill(v, &mut pre) };
                lowered.push(Arg::Value(v));
            }
        }
        let mut temps: HashMap<usize, Expr> = HashMap::new();
        let mut refs: HashMap<usize, Place> = HashMap::new();
        if !in_order {
            for (ai, a) in args.iter().enumerate() {
                let pi = order.iter().position(|&x| x == ai).expect("every argument is bound");
                if params[skip + pi].is_var {
                    let mut p = self.place(&a.value).expect("checker requires a place for a `var` argument");
                    self.stabilize(&mut p, &mut pre);
                    refs.insert(ai, p);
                } else {
                    let v = self.expr(&a.value);
                    temps.insert(ai, self.spill(v, &mut pre));
                }
            }
        }
        for (pi, &ai) in order.iter().enumerate() {
            let value = &args[ai].value;
            lowered.push(if let Some(p) = refs.remove(&ai) {
                Arg::Ref(p)
            } else if params[skip + pi].is_var {
                Arg::Ref(self.place(value).expect("checker requires a place for a `var` argument"))
            } else if let Some(v) = temps.remove(&ai) {
                Arg::Value(v)
            } else {
                Arg::Value(self.expr(value))
            });
        }
        (pre, FuncIdx(f.0), lowered)
    }

    /// Whether a call to `f` builds text in a scratch arena (see `wants_scratch` in the
    /// checker, which also treats the arguments as memory freed at the call's end).
    pub(crate) fn wants_scratch(&self, f: FnId, args: &[ast::Arg]) -> bool {
        kestrel_check::wants_scratch(self.t.func(f), args)
    }

    /// The pieces of a call to native `f` made inside a scratch arena: the statements
    /// that push the arena and evaluate the arguments, the call's own parts, and the
    /// statements that pop it. Leaving early from an argument pops it too (see `scratch`).
    fn scratch_parts(&mut self, e: &ast::Expr, f: FnId, args: &[ast::Arg]) -> (Vec<Stmt>, FuncIdx, Vec<Arg>, Vec<Stmt>) {
        let mut before = vec![Stmt::PushArena(None)];
        let ((pre, fi, lowered), after) = self.scratch(|this| this.call_parts(e, f, args, None));
        before.extend(pre);
        (before, fi, lowered, after)
    }

    /// A call to native `f` whose result is discarded, with its text in a scratch arena.
    pub(crate) fn scratch_call_stmt(&mut self, e: &ast::Expr, f: FnId, args: &[ast::Arg], out: &mut Vec<Stmt>) {
        let (before, fi, lowered, after) = self.scratch_parts(e, f, args);
        out.extend(before);
        out.push(Stmt::Call(fi, lowered));
        out.extend(after);
    }

    /// A call to native `f` that gives a value of type `ty`, with its text in a scratch
    /// arena. The result is kept in a temporary and read once the arena is popped. (A native
    /// gives back only scalars and `vec2`/`vec3`, which hold no arena memory.)
    pub(crate) fn scratch_call_value(&mut self, e: &ast::Expr, f: FnId, args: &[ast::Arg], ty: KTy, line: u32) -> ExprKind {
        let (mut stmts, fi, lowered, after) = self.scratch_parts(e, f, args);
        let result = self.temp(ty);
        stmts.push(Stmt::Let(result, Expr { kind: ExprKind::Call(fi, lowered), ty, line }));
        stmts.extend(after);
        ExprKind::Seq(stmts, Box::new(Expr { kind: ExprKind::Load(Place::local(result)), ty, line }))
    }

    /// `Point(...)`: every field in declaration order, defaults filled in,
    /// arguments evaluated in source order.
    pub(crate) fn construct(&mut self, e: &ast::Expr, sid: StructId, args: &[ast::Arg]) -> (Vec<Stmt>, ExprKind) {
        let (t, program) = (self.t, self.program);
        let slots = &t.ctor_args[&e.id];
        let bound: Vec<usize> = slots.iter().flatten().copied().collect();
        let in_order = bound.windows(2).all(|w| w[0] < w[1]);
        let mut pre = Vec::new();
        let mut temps: HashMap<usize, Expr> = HashMap::new();
        if !in_order {
            for (ai, a) in args.iter().enumerate() {
                let v = self.value(&a.value);
                temps.insert(ai, self.spill(v, &mut pre));
            }
        }
        let mut fields = Vec::with_capacity(slots.len());
        for (fi, slot) in slots.iter().enumerate() {
            fields.push(match slot {
                Some(ai) => match temps.remove(ai) {
                    Some(v) => v,
                    None => self.value(&args[*ai].value),
                },
                None => {
                    // Only user structs have defaults; built-in ones have no declaration.
                    let ast::Item::Struct(decl) = &program.items[t.strukt(sid).item] else { unreachable!("a struct with defaults") };
                    self.value(decl.fields[fi].default.as_ref().expect("checker requires a default for a missing field"))
                }
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
                let v = self.value(&a.value);
                values[ai] = Some(self.spill(v, &mut pre));
            }
        }
        let mut fields = Vec::with_capacity(slots.len());
        for slot in slots {
            let ai = slot.expect("checker binds every variant field");
            fields.push(match values[ai].take() {
                Some(v) => v,
                None => self.value(&args[ai].value),
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
