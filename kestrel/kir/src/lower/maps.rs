//! Maps: literals, `m[k]`, `m[k] = v`, `get`, `has`, `remove` and
//! `for k, v in m`. The runtime finds and adds entries; see `MapOp`.

use kestrel_syntax::ast;

use super::helpers::has_heap;
use super::{kty, FnLowerer};
use crate::ir::*;
use crate::rename::{rename_stmts, Subst};

fn map_expr(op: MapOp, map: Place, args: Vec<Expr>, ty: KTy, line: u32) -> Expr {
    Expr { kind: ExprKind::Map(op, map, args), ty, line }
}

fn load(l: LocalIdx, ty: KTy, line: u32) -> Expr {
    Expr { kind: ExprKind::Load(Place::local(l)), ty, line }
}

fn with(place: &Place, proj: Proj) -> Place {
    let mut p = place.clone();
    p.path.push(proj);
    p
}

impl<'a> FnLowerer<'a> {
    /// `{k: v, ...}`: a new map, then each entry set in order.
    pub(crate) fn map_lit(&mut self, entries: &[(ast::Expr, ast::Expr)], ty: KTy, line: u32) -> ExprKind {
        let tmp = self.temp(ty);
        let mut stmts = vec![Stmt::Let(tmp, Expr { kind: ExprKind::MapNew, ty, line })];
        for (k, v) in entries {
            let key = self.expr(k);
            let value = self.value(v);
            stmts.push(Stmt::MapSet(Place::local(tmp), key, value));
        }
        ExprKind::Seq(stmts, Box::new(Expr { kind: ExprKind::Move(Place::local(tmp)), ty, line }))
    }

    /// `m[k]`: an optional holding the value. When it holds arrays or
    /// lists, they're shared with the map, like a list element read from
    /// its place: storing the value copies it (see `owned`). A caller's map
    /// (a parameter or loop element) is copied from right away, since
    /// `owned` wouldn't know the value isn't this function's to hand out.
    pub(crate) fn map_get(&mut self, base: &ast::Expr, key: &ast::Expr, ty: KTy, line: u32) -> ExprKind {
        let mut pre = Vec::new();
        let map = self.place_or_temp(base, &mut pre, true);
        let key = self.expr(key);
        let borrowed = self.borrowed(&map);
        let get = map_expr(MapOp::Get, map, vec![key], ty, line);
        if !has_heap(ty, self.t) {
            return Self::with_pre(pre, get.kind, ty, line);
        }
        let tmp = self.temp(ty);
        pre.push(Stmt::Let(tmp, get));
        let kind = if borrowed {
            pre.extend(self.helpers.fixup(Place::local(tmp), ty, self.t));
            ExprKind::Move(Place::local(tmp))
        } else {
            ExprKind::Load(Place::local(tmp))
        };
        ExprKind::Seq(pre, Box::new(Expr { kind, ty, line }))
    }

    /// Whether the map at `map` belongs to a caller (it's reached through a
    /// parameter or a loop element).
    fn borrowed(&self, map: &Place) -> bool {
        self.locals[map.root.0 as usize].kind == LocalKind::RefParam
    }

    /// `m.get(k, d)`, `m.has(k)` and `m.remove(k)`, as values.
    pub(crate) fn map_method(&mut self, receiver: &ast::Expr, name: &str, args: &[ast::Arg], ty: KTy, line: u32) -> ExprKind {
        let mut pre = Vec::new();
        let map = self.place_or_temp(receiver, &mut pre, true);
        let key = self.expr(&args[0].value);
        let op = match name {
            "has" => MapOp::Has,
            "remove" => MapOp::Remove,
            _ => {
                let KTy::Map(_, v) = kty(self.t.ty(receiver.id)) else { unreachable!("`get` on a map") };
                return self.map_get_or(map, key, &args[1].value, KTy::Optional(v), line, pre);
            }
        };
        Self::with_pre(pre, ExprKind::Map(op, map, vec![key]), ty, line)
    }

    /// `m.get(k, d)`: the value when `k` is there, else `d`. The key and the
    /// default are evaluated first, in that order.
    fn map_get_or(&mut self, map: Place, key: Expr, default: &ast::Expr, opt_ty: KTy, line: u32, mut pre: Vec<Stmt>) -> ExprKind {
        let key = self.spill(key, &mut pre);
        let default = self.value(default);
        let ty = default.ty;
        let default = self.spill(default, &mut pre);
        let opt = self.temp(opt_ty);
        let borrowed = self.borrowed(&map);
        pre.push(Stmt::Let(opt, map_expr(MapOp::Get, map, vec![key], opt_ty, line)));
        if borrowed && has_heap(opt_ty, self.t) {
            pre.extend(self.helpers.fixup(Place::local(opt), opt_ty, self.t));
        }
        let tag = Expr { kind: ExprKind::Tag(Place::local(opt)), ty: KTy::U8, line };
        let zero = Expr { kind: ExprKind::Int(0), ty: KTy::U8, line };
        let found = Expr { kind: ExprKind::Binary(BinOp::Ne, Box::new(tag), Box::new(zero)), ty: KTy::Bool, line };
        let value = Expr { kind: ExprKind::Load(with(&Place::local(opt), Proj::Payload)), ty, line };
        let kind = ExprKind::If {
            cond: Box::new(found),
            then_body: Vec::new(),
            then_val: Some(Box::new(value)),
            else_body: Vec::new(),
            else_val: Some(Box::new(default)),
        };
        Self::with_pre(pre, kind, ty, line)
    }

    /// `m[k] = v`: the key, then the value, then the map changes.
    pub(crate) fn map_set(&mut self, base: &ast::Expr, key: &ast::Expr, value: &ast::Expr, out: &mut Vec<Stmt>) {
        let map = self.place_or_temp(base, out, true);
        let key = self.expr(key);
        let value = self.value(value);
        out.push(Stmt::MapSet(map, key, value));
    }

    /// `for k, v in m` / `for k in m`: a counted loop over the entries,
    /// skipping removed ones. When the map could gain keys meanwhile, it's
    /// locked, so it can't. `v` stands for the entry's value like a list
    /// loop's element.
    pub(crate) fn map_for_in(
        &mut self,
        key: Option<&ast::Binding>,
        var: &ast::Binding,
        iter: &ast::Expr,
        body: &ast::Block,
        out: &mut Vec<Stmt>,
    ) {
        let line = self.line(iter.span);
        let map = self.place_or_temp(iter, out, true);
        let locked = self.t.locked_map_loops.contains(&iter.id);
        let saved = self.temp(KTy::I64);
        if locked {
            out.push(Stmt::Let(saved, map_expr(MapOp::Lock, map.clone(), Vec::new(), KTy::I64, line)));
        }
        let i = self.temp(KTy::I64);
        let at = load(i, KTy::I64, line);
        let (key, value) = match key {
            Some(k) => (k, Some(var)),
            None => (var, None),
        };
        let key = self.bind_local(key);
        let key_ty = self.locals[key.0 as usize].ty;
        let mut stmts = vec![Stmt::Let(key, Expr { kind: ExprKind::Load(with(&map, Proj::MapKey { at: at.clone() })), ty: key_ty, line })];
        let value_ref = value.map(|var| {
            let var = self.t.bind_local[&var.id];
            let ptr = LocalIdx(self.locals.len() as u32);
            let ty = kty(self.t.local(var).ty);
            self.locals.push(Local { name: format!("{}_ref", self.t.local(var).name), ty, kind: LocalKind::RefParam });
            let checked = with(&map, Proj::MapValue { at: at.clone(), checked: true });
            self.origins.insert(ptr, checked.clone());
            self.elems.insert(var, Place::local(ptr));
            stmts.push(Stmt::Bind(ptr, with(&map, Proj::MapValue { at: at.clone(), checked: false })));
            (ptr, checked)
        });
        let body_stmts = if locked { self.locked(map.clone(), saved, |l| l.loop_block(body)) } else { self.loop_block(body) };
        stmts.extend(body_stmts);
        if let Some((ptr, checked)) = value_ref {
            if self.may_move(&stmts, &map) {
                // The body may replace the map: find the value again on every use.
                let at = stmts.iter().position(|s| matches!(s, Stmt::Bind(p, _) if *p == ptr)).expect("bound above");
                stmts.remove(at);
                let mut subst: Vec<Subst> = (0..self.locals.len() as u32).map(|l| Subst::Local(LocalIdx(l))).collect();
                subst[ptr.0 as usize] = Subst::Place(checked);
                rename_stmts(&mut stmts, &subst);
            }
        }
        let live = map_expr(MapOp::Live, map.clone(), vec![at], KTy::Bool, line);
        let start = Expr { kind: ExprKind::Int(0), ty: KTy::I64, line };
        let end = map_expr(MapOp::Used, map.clone(), Vec::new(), KTy::I64, line);
        out.push(Stmt::ForRange { var: i, start, end, body: vec![Stmt::If { cond: live, then_body: stmts, else_body: Vec::new() }] });
        if locked {
            out.push(Stmt::MapUnlock(map, load(saved, KTy::I64, line)));
        }
    }
}
