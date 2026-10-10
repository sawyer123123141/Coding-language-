//! Generated functions for maps: deep copies and printing (see `helpers`).

use kestrel_check::Tables;

use super::helpers::{expr, has_heap, with, Helpers};
use super::kty;
use crate::ir::*;

/// The key and value types of a map type.
fn key_value(ty: KTy, t: &Tables) -> (KTy, KTy) {
    let KTy::Map(k, v) = ty else { unreachable!("{ty:?} isn't a map") };
    (kty(t.elems[k.0 as usize]), kty(t.elems[v.0 as usize]))
}

/// `for i from 0 to m.used { if live(i) { body(key, value) } }`, over the
/// map in local 0 with local 1 as `i`.
fn each_entry(body: impl FnOnce(Place, Place) -> Vec<Stmt>) -> Stmt {
    let (m, i) = (Place::local(LocalIdx(0)), LocalIdx(1));
    let at = expr(ExprKind::Load(Place::local(i)), KTy::I64);
    let key = with(&m, Proj::MapKey { at: at.clone() });
    // The body never adds or removes keys, so the entry stays put.
    let value = with(&m, Proj::MapValue { at: at.clone(), checked: false });
    let live = expr(ExprKind::Map(MapOp::Live, m.clone(), vec![at]), KTy::Bool);
    Stmt::ForRange {
        var: i,
        start: expr(ExprKind::Int(0), KTy::I64),
        end: expr(ExprKind::Map(MapOp::Used, m, Vec::new()), KTy::I64),
        body: vec![Stmt::If { cond: live, then_body: body(key, value), else_body: Vec::new() }],
    }
}

fn text(s: &str) -> PrintArg {
    PrintArg::Str(s.to_string())
}

/// The value at `p` for printing inside a collection: strings quoted.
fn shown(p: Place, ty: KTy) -> Vec<PrintArg> {
    let item = PrintArg::Value(expr(ExprKind::Load(p), ty));
    if ty == KTy::Str {
        vec![text("\""), item, text("\"")]
    } else {
        vec![item]
    }
}

impl Helpers {
    /// `copy(var m)`: fresh, compact entries for `m`, then each key's and
    /// value's own copies.
    pub(crate) fn map_copy_fn(&mut self, ty: KTy, t: &Tables) -> FuncIdx {
        if let Some(&f) = self.copy.get(&ty) {
            return f;
        }
        let idx = self.reserve();
        self.copy.insert(ty, idx);
        let (k, v) = key_value(ty, t);
        let mut body = vec![Stmt::Dup(Place::local(LocalIdx(0)))];
        if has_heap(k, t) || has_heap(v, t) {
            body.push(each_entry(|key, value| {
                let mut copies = self.fixup(key, k, t);
                copies.extend(self.fixup(value, v, t));
                copies
            }));
        }
        self.funcs[(idx.0 - self.base) as usize] = Func {
            name: format!("__kestrel_copy_{}", idx.0),
            params: vec![LocalIdx(0)],
            ret: None,
            locals: Self::locals(ty),
            body,
            is_pure: true,
            io: false,
            mutates_params: true,
            exported: false,
            native: None,
        };
        idx
    }

    /// `print(m)` without a newline: `{"a": 1, "b": 2}`, strings quoted.
    pub(crate) fn map_print_fn(&mut self, ty: KTy, t: &Tables) -> FuncIdx {
        if let Some(&f) = self.print.get(&ty) {
            return f;
        }
        let idx = self.reserve();
        self.print.insert(ty, idx);
        let (k, v) = key_value(ty, t);
        self.ensure_print(k, t);
        self.ensure_print(v, t);
        // Local 2: whether an entry was printed already (removed entries print nothing).
        let started = Place::local(LocalIdx(2));
        let loop_ = each_entry(|key, value| {
            let mut piece = shown(key, k);
            piece.push(text(": "));
            piece.extend(shown(value, v));
            vec![
                Stmt::If {
                    cond: expr(ExprKind::Load(started.clone()), KTy::Bool),
                    then_body: vec![Stmt::Write(vec![text(", ")])],
                    else_body: Vec::new(),
                },
                Stmt::Write(piece),
                Stmt::Assign(started.clone(), expr(ExprKind::Bool(true), KTy::Bool)),
            ]
        });
        let body = vec![
            Stmt::Let(LocalIdx(2), expr(ExprKind::Bool(false), KTy::Bool)),
            Stmt::Write(vec![text("{")]),
            loop_,
            Stmt::Write(vec![text("}")]),
        ];
        let mut locals = Self::locals(ty);
        locals.push(Local { name: "started".into(), ty: KTy::Bool, kind: LocalKind::Value });
        self.funcs[(idx.0 - self.base) as usize] = Func {
            name: format!("__kestrel_print_{}", idx.0),
            params: vec![LocalIdx(0)],
            ret: None,
            locals,
            body,
            is_pure: false,
            io: true,
            mutates_params: false,
            exported: false,
            native: None,
        };
        idx
    }
}
