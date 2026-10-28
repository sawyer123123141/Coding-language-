//! Storage-reusing assignment (Rust's `clone_from`): the generated helpers that assign the value at
//! one place to another, copying into the target's buffers when they have room. Lowering calls
//! `__kestrel_assign_N(var dst: T, src: T)` for a plain `target = value` that may reuse (see
//! `assign_site`); the two places never share memory there.
//!
//! - A list or array with room (`src.len <= Cap(dst)`: a list's capacity, an array's length) takes
//!   plain items in one copy (`CopyItems`). Items holding heap memory are assigned one by one over
//!   `0 .. min(old len, new len)`, so their own buffers are reused, and copied fresh past the old
//!   length (those slots hold nothing of the target's any more). Without room it is exactly
//!   today's copy, a fresh buffer in the current arena: reuse never grows a buffer, so memory goes
//!   to the same arenas as before and the checker's escape rules stay sufficient.
//! - A string is never written into (it may be a literal in read-only data): one whose bytes equal
//!   the new value's is kept, any other is copied fresh.
//! - A struct goes field by field; an optional or enum reuses its payload when both sides hold the
//!   same variant, and otherwise is copied as today.
//! - A map that isn't being looped over and has room (entries and an index table for the new
//!   value's live keys) takes a compact copy of the entries in place (`MapOp::CopyInto`), keeping
//!   its arena; then heap keys and values get fresh copies over the entries (the map's own buffers
//!   are reused, its keys' and values' aren't). A map being looped over, or without room, is copied
//!   as today, so the loop sees a new map, as it always has.
//! - A function value and an interface value are copied as today.
//!
//! Today's copy for a list, array or map without room keeps its temporary under a name starting
//! with `ROOM_COPY`, so the report says it is the assignment's copy when there is too little room.

use kestrel_check::Tables;

use super::helpers::{elem_of, expr, has_heap, has_value, tag_is, with, Helpers};
use super::kty;
use super::map_helpers::{each_entry_of, key_value};
use crate::ir::*;

/// Whether a value of `ty` holds a list, array, map or string whose storage an assignment can
/// reuse. Function values and interface values are copied as today, so a type holding only those
/// gains nothing.
pub(crate) fn reusable(ty: KTy, t: &Tables) -> bool {
    reusable_below(ty, t, &mut Vec::new())
}

/// `reusable`, skipping the types in `seen` (a layout walk must end even if the checker let a
/// type that holds itself through).
fn reusable_below(ty: KTy, t: &Tables, seen: &mut Vec<KTy>) -> bool {
    match ty {
        KTy::Str | KTy::Array(_) | KTy::List(_) | KTy::Map(..) => true,
        KTy::Optional(e) => reusable_below(kty(t.elems[e.0 as usize]), t, seen),
        KTy::Struct(_) | KTy::Enum(_) if seen.contains(&ty) => false,
        KTy::Struct(s) => {
            seen.push(ty);
            t.structs[s.0 as usize].fields.iter().any(|f| reusable_below(kty(f.ty), t, seen))
        }
        KTy::Enum(e) => {
            seen.push(ty);
            t.enums[e.0 as usize].variants.iter().flat_map(|v| &v.fields).any(|f| reusable_below(kty(f.ty), t, seen))
        }
        _ => false,
    }
}

fn load(p: Place, ty: KTy) -> Expr {
    expr(ExprKind::Load(p), ty)
}

fn local(l: u32, ty: KTy) -> Expr {
    load(Place::local(LocalIdx(l)), ty)
}

/// `place[at]`, known in bounds.
fn item(place: &Place, at: LocalIdx) -> Place {
    with(place, Proj::Index { at: local(at.0, KTy::I64), check: IndexCheck::Known })
}

fn binary(op: BinOp, a: Expr, b: Expr, ty: KTy) -> Expr {
    expr(ExprKind::Binary(op, Box::new(a), Box::new(b)), ty)
}

// The locals of an assign helper for a list or array (a map's has `dst`, `src` and `ENTRY`).
const DST: u32 = 0;
const SRC: u32 = 1;
const OLD: u32 = 2;
const NEW: u32 = 3;
const MIN: u32 = 4;
const I: u32 = 5;
const J: u32 = 6;
const ENTRY: u32 = 2;

impl Helpers {
    /// `__kestrel_assign_N(var dst: T, src: T)`: assigns `src` to `dst`, reusing `dst`'s storage
    /// where it can. One per type; a list, array or map type's holds the loops, every other type's
    /// is `assign_stmts` inline (and calls the list and map helpers for its lists and maps).
    pub(crate) fn assign_fn(&mut self, ty: KTy, t: &Tables) -> FuncIdx {
        if let Some(&f) = self.assign.get(&ty) {
            return f;
        }
        let idx = self.reserve();
        self.assign.insert(ty, idx);
        let mut locals = vec![
            Local { name: "dst".into(), ty, kind: LocalKind::RefParam },
            Local { name: "src".into(), ty, kind: LocalKind::RefParam },
        ];
        let numbers: &[&str] = match ty {
            KTy::Array(_) | KTy::List(_) => &["old", "new", "min", "i", "j"],
            KTy::Map(..) => &["entry"],
            _ => &[],
        };
        for name in numbers {
            locals.push(Local { name: (*name).into(), ty: KTy::I64, kind: LocalKind::Value });
        }
        // (A helper built while this one is, for a list inside it, keeps its own locals.)
        let outer = std::mem::replace(&mut self.assign_locals, locals);
        let (dst, src) = (Place::local(LocalIdx(DST)), Place::local(LocalIdx(SRC)));
        let body = match ty {
            KTy::Array(_) | KTy::List(_) => self.items_body(ty, t),
            KTy::Map(..) => self.map_body(ty, t),
            _ => self.assign_stmts(dst, src, ty, t),
        };
        let locals = std::mem::replace(&mut self.assign_locals, outer);
        self.funcs[(idx.0 - self.base) as usize] = Func {
            name: format!("__kestrel_assign_{}", idx.0),
            params: vec![LocalIdx(DST), LocalIdx(SRC)],
            ret: None,
            locals,
            body,
            is_pure: true,
            io: false,
            mutates_params: true,
            exported: false,
            lines: (0, 0),
            native: None,
        };
        idx
    }

    /// The body of a list or array's assign helper (see the module comment).
    fn items_body(&mut self, ty: KTy, t: &Tables) -> Vec<Stmt> {
        let (dst, src) = (Place::local(LocalIdx(DST)), Place::local(LocalIdx(SRC)));
        let elem = elem_of(ty, t);
        let mut body = vec![Stmt::Let(LocalIdx(NEW), expr(ExprKind::Len(src.clone()), KTy::I64))];
        let room = binary(BinOp::Le, local(NEW, KTy::I64), expr(ExprKind::Cap(dst.clone()), KTy::I64), KTy::Bool);
        let then_body = if has_heap(elem, t) {
            body.insert(0, Stmt::Let(LocalIdx(OLD), expr(ExprKind::Len(dst.clone()), KTy::I64)));
            let fewer = binary(BinOp::Lt, local(OLD, KTy::I64), local(NEW, KTy::I64), KTy::Bool);
            let min = ExprKind::If {
                cond: Box::new(fewer),
                then_body: Vec::new(),
                then_val: Some(Box::new(local(OLD, KTy::I64))),
                else_body: Vec::new(),
                else_val: Some(Box::new(local(NEW, KTy::I64))),
            };
            // `i < min(old, new)`: within both lengths (the target's is `new` from here on).
            let (at_i, at_j) = (LocalIdx(I), LocalIdx(J));
            let reused = self.assign_stmts(item(&dst, at_i), item(&src, at_i), elem, t);
            // `min <= j < new`: past the old length the target's slots hold no live item (they are
            // zero, or stale items nothing reads), so each is written whole with a fresh copy and
            // never read first.
            let fresh = self.copied(item(&dst, at_j), item(&src, at_j), elem, t);
            vec![
                Stmt::SetLen(dst.clone(), local(NEW, KTy::I64)),
                Stmt::Let(LocalIdx(MIN), expr(min, KTy::I64)),
                Stmt::ForRange { var: at_i, start: expr(ExprKind::Int(0), KTy::I64), end: local(MIN, KTy::I64), body: reused },
                Stmt::ForRange { var: at_j, start: local(MIN, KTy::I64), end: local(NEW, KTy::I64), body: fresh },
            ]
        } else {
            vec![Stmt::CopyItems(dst.clone(), src.clone())]
        };
        body.push(Stmt::If { cond: room, then_body, else_body: self.copied_without_room(dst, src, ty, t) });
        body
    }

    /// The body of a map's assign helper (see the module comment).
    fn map_body(&mut self, ty: KTy, t: &Tables) -> Vec<Stmt> {
        let (dst, src) = (Place::local(LocalIdx(DST)), Place::local(LocalIdx(SRC)));
        let copied_into = expr(ExprKind::Map(MapOp::CopyInto, dst.clone(), vec![load(src.clone(), ty)]), KTy::Bool);
        let (k, v) = key_value(ty, t);
        // The entries hold `src`'s keys and values as bytes now: heap ones get their own copies, so
        // nothing of `dst` points into `src`. (After `CopyInto` every entry is live.)
        let then_body = if has_heap(k, t) || has_heap(v, t) {
            vec![each_entry_of(dst.clone(), LocalIdx(ENTRY), |key, value| {
                let mut copies = self.fixup(key, k, t);
                copies.extend(self.fixup(value, v, t));
                copies
            })]
        } else {
            Vec::new()
        };
        vec![Stmt::If { cond: copied_into, then_body, else_body: self.copied_without_room(dst, src, ty, t) }]
    }

    /// Today's copy of `src` into `dst`, as `owned` makes it: `src` into a temporary, which gets its
    /// own copies of what it holds (in the current arena), then into `dst`. (So a contract's chain
    /// names the copy as it always has: "makes a copy of `v`".)
    fn copied(&mut self, dst: Place, src: Place, ty: KTy, t: &Tables) -> Vec<Stmt> {
        self.copied_through("tmp", dst, src, ty, t)
    }

    /// `copied`, for a list, array or map whose own storage has too little room (or a map being
    /// looped over): the temporary's name says so, and the report words the copy as the
    /// assignment's.
    fn copied_without_room(&mut self, dst: Place, src: Place, ty: KTy, t: &Tables) -> Vec<Stmt> {
        self.copied_through(ROOM_COPY, dst, src, ty, t)
    }

    /// `copied`, through a temporary whose name starts with `name`.
    fn copied_through(&mut self, name: &str, dst: Place, src: Place, ty: KTy, t: &Tables) -> Vec<Stmt> {
        let tmp = LocalIdx(self.assign_locals.len() as u32);
        self.assign_locals.push(Local { name: format!("{name}{}", tmp.0), ty, kind: LocalKind::Addressed });
        let mut out = vec![Stmt::Let(tmp, load(src, ty))];
        out.extend(self.fixup(Place::local(tmp), ty, t));
        out.push(Stmt::Assign(dst, load(Place::local(tmp), ty)));
        out
    }

    /// Statements that assign the value at `src` to `dst` (places that share no memory), reusing
    /// `dst`'s storage where it has room (see the module comment).
    pub(crate) fn assign_stmts(&mut self, dst: Place, src: Place, ty: KTy, t: &Tables) -> Vec<Stmt> {
        match ty {
            _ if !has_heap(ty, t) => vec![Stmt::Assign(dst, load(src, ty))],
            KTy::Str => {
                let differ = binary(BinOp::Ne, load(dst.clone(), ty), load(src.clone(), ty), KTy::Bool);
                vec![Stmt::If { cond: differ, then_body: self.copied(dst, src, ty, t), else_body: Vec::new() }]
            }
            KTy::Array(_) | KTy::List(_) | KTy::Map(..) => vec![Stmt::Call(self.assign_fn(ty, t), vec![Arg::Ref(dst), Arg::Ref(src)], 0)],
            KTy::Optional(e) => {
                let both = expr(ExprKind::And(Box::new(has_value(&dst)), Box::new(has_value(&src))), KTy::Bool);
                let payload = kty(t.elems[e.0 as usize]);
                let then_body = self.assign_stmts(with(&dst, Proj::Payload), with(&src, Proj::Payload), payload, t);
                vec![Stmt::If { cond: both, then_body, else_body: self.copied(dst, src, ty, t) }]
            }
            KTy::Struct(s) => {
                let mut out = Vec::new();
                for (i, f) in t.structs[s.0 as usize].fields.iter().enumerate() {
                    let at = |p: &Place| with(p, Proj::Field(i as u32));
                    out.extend(self.assign_stmts(at(&dst), at(&src), kty(f.ty), t));
                }
                out
            }
            KTy::Enum(e) => {
                let tag = |p: &Place| expr(ExprKind::Tag(p.clone()), KTy::U8);
                let other = binary(BinOp::Ne, tag(&dst), tag(&src), KTy::Bool);
                let mut same = Vec::new();
                for (v, variant) in t.enums[e.0 as usize].variants.iter().enumerate() {
                    let mut fields = Vec::new();
                    for (i, f) in variant.fields.iter().enumerate() {
                        let at = |p: &Place| with(p, Proj::Variant(v as u32, i as u32));
                        fields.extend(self.assign_stmts(at(&dst), at(&src), kty(f.ty), t));
                    }
                    if !fields.is_empty() {
                        same.push(Stmt::If { cond: tag_is(&src, v as u32), then_body: fields, else_body: Vec::new() });
                    }
                }
                vec![Stmt::If { cond: other, then_body: self.copied(dst, src, ty, t), else_body: same }]
            }
            // Function values and interface values.
            _ => self.copied(dst, src, ty, t),
        }
    }
}
