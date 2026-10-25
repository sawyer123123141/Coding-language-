use std::collections::HashMap;

use kestrel_check::{EnumId, FnId, Res, StructId};
use kestrel_syntax::ast;

use super::consts::hoisted;
use super::FnLowerer;
use crate::ir::*;

impl<'a> FnLowerer<'a> {
    /// The place an expression names (`x`, `x.f.g`, `xs[i]`), if it is one.
    pub(crate) fn place(&mut self, e: &ast::Expr) -> Option<Place> {
        // A closure's read of a capture: its place in the environment.
        if let Some(p) = self.capture_place(e.id) {
            return Some(p);
        }
        match &e.kind {
            ast::ExprKind::Ident(_) => match self.res(e.id) {
                Some(Res::Local(l)) => Some(match self.elems.get(&l) {
                    Some(p) => p.clone(),
                    None => Place::local(self.local_map[&l]),
                }),
                // A heap constant is a place that is only read: never moved, so storing it copies.
                Some(Res::Const(c)) if hoisted(self.t, c) => Some(Place::local(self.const_local(c))),
                _ => None,
            },
            // `m[k]` is a lookup, and `s[i]` builds a new string: neither is a place.
            ast::ExprKind::Index { base, .. } if matches!(self.ty(base.id), kestrel_check::Ty::Map(..) | kestrel_check::Ty::Str) => None,
            ast::ExprKind::Index { base, index } => {
                let mut p = self.place(base)?;
                p.path.push(Proj::Index { at: self.expr(index), check: IndexCheck::Checked });
                Some(p)
            }
            // A lane of a vector place, the place's last step.
            ast::ExprKind::Field { base, .. } if self.t.lanes.contains_key(&e.id) => {
                let mut p = self.place(base)?;
                p.path.push(Proj::Lane(self.t.lanes[&e.id] as u8));
                Some(p)
            }
            ast::ExprKind::Field { base, .. } => {
                // `Enum.Variant` parses as a field access but isn't a place.
                let field = *self.t.field_index.get(&e.id)?;
                if let Some(Res::Local(l)) = self.res(base.id) {
                    if let Some((p, variant)) = self.bindings.get(&l) {
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
                let v = if in_order { self.held(v) } else { self.spill_held(v, &mut pre) };
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
                    let v = self.arg_in_turn(v, &mut pre);
                    temps.insert(ai, v);
                }
            }
        }
        for (pi, &ai) in order.iter().enumerate() {
            let value = &args[ai].value;
            lowered.push(if let Some(p) = refs.remove(&ai) {
                self.var_arg(value, p)
            } else if params[skip + pi].is_var {
                let p = self.place(value).expect("checker requires a place for a `var` argument");
                self.var_arg(value, p)
            } else if let Some(v) = temps.remove(&ai) {
                Arg::Value(v)
            } else {
                let v = self.expr(value);
                Arg::Value(self.held(v))
            });
        }
        // Last of `pre`, so it runs after the arguments' own statements and right before the call.
        self.record_line(e, f, &mut lowered, &mut pre);
        (pre, FuncIdx(f.0), lowered)
    }

    /// Whether a call to `f` builds text in a scratch arena (see `wants_scratch` in the
    /// checker, which also treats the arguments as memory freed at the call's end).
    pub(crate) fn wants_scratch(&self, f: FnId, args: &[ast::Arg]) -> bool {
        kestrel_check::wants_scratch(self.t.func(f), args, self.t)
    }

    /// The pieces of a call to native `f` made inside a scratch arena: the statements
    /// that push the arena and evaluate the arguments, the call's own parts, and the
    /// statements that pop it. Leaving early from an argument pops it too (see `scratch`).
    fn scratch_parts(&mut self, e: &ast::Expr, f: FnId, args: &[ast::Arg]) -> (Vec<Stmt>, FuncIdx, Vec<Arg>, Vec<Stmt>) {
        let mut before = vec![Stmt::PushArena(None, 0)];
        let ((pre, fi, lowered), after) = self.scratch(|this| this.call_parts(e, f, args, None));
        before.extend(pre);
        (before, fi, lowered, after)
    }

    /// A call to native `f` whose result is discarded, with its text in a scratch arena.
    pub(crate) fn scratch_call_stmt(&mut self, e: &ast::Expr, f: FnId, args: &[ast::Arg], line: u32, out: &mut Vec<Stmt>) {
        let (before, fi, lowered, after) = self.scratch_parts(e, f, args);
        out.extend(before);
        // The scratch arena is popped, so a result that is held goes to the block around the call.
        let ret = self.t.func(f).ret;
        if self.t.is_owning(ret) {
            let result = self.temp(super::kty(ret));
            out.push(Stmt::Let(result, Expr { kind: ExprKind::Call(fi, lowered), ty: super::kty(ret), line }));
            out.extend(after);
            self.declared(result);
        } else {
            out.push(Stmt::Call(fi, lowered, line));
            out.extend(after);
        }
    }

    /// A call to native `f` that gives a value of type `ty`, with its text in a scratch
    /// arena. The result is kept in a temporary and read once the arena is popped. (A native
    /// gives back a number, a bool, a vector, or a struct of those that may hold a `GpuHandle`.
    /// None holds scratch memory; a struct with a `GpuHandle` is an owning value, moved out of
    /// the temporary.)
    pub(crate) fn scratch_call_value(&mut self, e: &ast::Expr, f: FnId, args: &[ast::Arg], ty: KTy, line: u32) -> ExprKind {
        let (mut stmts, fi, lowered, after) = self.scratch_parts(e, f, args);
        let result = self.temp(ty);
        stmts.push(Stmt::Let(result, Expr { kind: ExprKind::Call(fi, lowered), ty, line }));
        stmts.extend(after);
        let place = Place::local(result);
        let value = if super::helpers::owning(ty, self.t) {
            self.scratch_results.insert(result);
            ExprKind::Move(place)
        } else {
            ExprKind::Load(place)
        };
        ExprKind::Seq(stmts, Box::new(Expr { kind: value, ty, line }))
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
                None => match program.items.get(t.strukt(sid).item) {
                    Some(ast::Item::Struct(decl)) => {
                        self.value(decl.fields[fi].default.as_ref().expect("checker requires a default for a missing field"))
                    }
                    // `vec2()` and `ivec3()`: a built-in vector's components start at zero.
                    _ => super::builtins::number(0, super::kty(t.strukt(sid).fields[fi].ty), self.line(e.span)),
                },
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
