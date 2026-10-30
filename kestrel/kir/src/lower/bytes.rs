//! `to_bytes(x)`: the value in the byte format of the I/O round's spec (decision 2; the runtime's
//! `bytes` has the format and a reference encoder). A generated function per type,
//! `__kestrel_to_bytes_N(x: T) -> list[u8]`, sizes the value, makes one `list[u8]` of exactly that
//! many bytes in the current arena (`ListOp::Reserve`, then `SetLen`), writes the 13-byte header
//! (`KESB`, the version, the type's `fingerprint`) and then the value:
//!
//! - `__kestrel_bytes_size_N(x: T) -> i64` gives the value's size without the header, and
//!   `__kestrel_bytes_write_N(var dst: list[u8], at: i64, x: T) -> i64` writes it into `dst` from
//!   byte `at` and gives the byte after it. Numbers, bools and vectors are one `WriteBytes`, a
//!   string one `WriteBytes` of its length and bytes, a length, live count or variant number one
//!   `WriteCount`.
//! - A struct is its fields in order (`vec2`, `vec3`, `ivec2` and `ivec3` are structs: their
//!   lanes), an enum its variant number then that variant's fields, an optional a 0, or a 1 then
//!   its payload; these are written inline. An array or list is its length then its first
//!   `length` items, a map its live count then each live key and value in insertion order; these
//!   always go through their own functions, so types that contain themselves through a collection
//!   work, as `checksum`'s do.
//!
//! Sizes add with wrapping (a value is never near `i64::MAX` bytes; the list's room fails first
//! past `MAX_ITEMS`), and writes are never checked: the list's length covers them. The checker
//! refuses values holding arenas, GPU objects, interface values or function values
//! (`kestrel_check::snapshot`).

use kestrel_check::Tables;
use kestrel_syntax::ast;

use super::helpers::{elem_of, expr, has_value, tag_is, with, Helpers};
use super::map_helpers::key_value;
use super::{kty, FnLowerer};
use crate::ir::*;

pub(crate) use super::bytes_fingerprint::fingerprint;

/// The header's length: the magic, the version and the fingerprint.
pub(crate) const HEADER: i64 = 13;
/// The magic `KESB`, as the `int` whose little-endian bytes it is.
const MAGIC: i64 = i32::from_le_bytes(*b"KESB") as i64;
/// The format version.
const VERSION: i64 = 1;

// The locals of a size function: the value, the size so far and a loop index.
const S_X: LocalIdx = LocalIdx(0);
const S_ACC: LocalIdx = LocalIdx(1);
const S_I: LocalIdx = LocalIdx(2);
// The locals of a write function: the list, the next byte, the value and a loop index.
const W_DST: LocalIdx = LocalIdx(0);
const W_AT: LocalIdx = LocalIdx(1);
const W_X: LocalIdx = LocalIdx(2);
const W_I: LocalIdx = LocalIdx(3);
// The locals of a `to_bytes` function: the value, the whole size and the result.
const T_X: LocalIdx = LocalIdx(0);
const T_N: LocalIdx = LocalIdx(1);
const T_OUT: LocalIdx = LocalIdx(2);

/// Whether `WriteBytes` writes a value of type `ty` whole, and how many bytes (a string's are its
/// length's four and its own, so `None` there).
fn word_size(ty: KTy) -> Option<i64> {
    match ty {
        KTy::Bool | KTy::U8 => Some(1),
        KTy::I32 | KTy::F32 => Some(4),
        KTy::I64 | KTy::F64 => Some(8),
        KTy::F32X4 | KTy::I32X4 => Some(16),
        _ => None,
    }
}

/// The size of every value of type `ty`, when they all have one: words, and structs of them.
fn fixed_size(ty: KTy, t: &Tables) -> Option<i64> {
    match ty {
        KTy::Struct(s) => t.structs[s.0 as usize].fields.iter().map(|f| fixed_size(kty(f.ty), t)).sum(),
        _ => word_size(ty),
    }
}

fn int(n: i64, ty: KTy) -> Expr {
    expr(ExprKind::Int(n), ty)
}

fn load(p: Place, ty: KTy) -> Expr {
    expr(ExprKind::Load(p), ty)
}

fn local(l: LocalIdx) -> Expr {
    load(Place::local(l), KTy::I64)
}

fn wrapping(op: BinOp, a: Expr, b: Expr) -> Expr {
    expr(ExprKind::Binary(op, Box::new(a), Box::new(b)), KTy::I64)
}

/// `l = l +% n`
fn add_to(l: LocalIdx, n: Expr) -> Stmt {
    Stmt::Assign(Place::local(l), wrapping(BinOp::AddWrap, local(l), n))
}

/// The length (or a map's live count) of the value at `p`, widened to an `i64`.
fn len(p: &Place) -> Expr {
    expr(ExprKind::Len(p.clone()), KTy::I64)
}

/// `value` written at the write function's next byte, which then moves past it by `size`.
fn put(op: ListOp, value: Expr, size: Expr) -> Vec<Stmt> {
    vec![Stmt::List(op, Place::local(W_DST), vec![local(W_AT), value]), add_to(W_AT, size)]
}

/// A parameter's kind: an aggregate is passed by pointer.
fn param_kind(ty: KTy) -> LocalKind {
    if ty.is_aggregate() {
        LocalKind::RefParam
    } else {
        LocalKind::Value
    }
}

fn number(name: &str) -> Local {
    Local { name: name.into(), ty: KTy::I64, kind: LocalKind::Value }
}

/// A generated function's frame: pure, with no lines.
fn helper(name: String, params: Vec<LocalIdx>, ret: KTy, locals: Vec<Local>, body: Vec<Stmt>, mutates_params: bool) -> Func {
    Func { name, params, ret: Some(ret), locals, body, is_pure: true, io: false, mutates_params, exported: false, lines: (0, 0), native: None }
}

impl Helpers {
    /// Statements that add the size of the value at `p`, of type `ty`, to the local `S_ACC`.
    fn size_stmts(&mut self, p: Place, ty: KTy, t: &Tables) -> Vec<Stmt> {
        if let Some(n) = fixed_size(ty, t) {
            return vec![add_to(S_ACC, int(n, KTy::I64))];
        }
        match ty {
            KTy::Str => vec![add_to(S_ACC, wrapping(BinOp::AddWrap, int(4, KTy::I64), len(&p)))],
            KTy::Struct(s) => {
                let mut out = Vec::new();
                for (i, f) in t.structs[s.0 as usize].fields.iter().enumerate() {
                    out.extend(self.size_stmts(with(&p, Proj::Field(i as u32)), kty(f.ty), t));
                }
                out
            }
            KTy::Enum(e) => {
                let mut out = vec![add_to(S_ACC, int(4, KTy::I64))];
                for (v, variant) in t.enums[e.0 as usize].variants.iter().enumerate() {
                    let mut then_body = Vec::new();
                    for (i, f) in variant.fields.iter().enumerate() {
                        then_body.extend(self.size_stmts(with(&p, Proj::Variant(v as u32, i as u32)), kty(f.ty), t));
                    }
                    if !then_body.is_empty() {
                        out.push(Stmt::If { cond: tag_is(&p, v as u32), then_body, else_body: Vec::new() });
                    }
                }
                out
            }
            KTy::Optional(e) => {
                let then_body = self.size_stmts(with(&p, Proj::Payload), kty(t.elems[e.0 as usize]), t);
                vec![add_to(S_ACC, int(1, KTy::I64)), Stmt::If { cond: has_value(&p), then_body, else_body: Vec::new() }]
            }
            KTy::Array(_) | KTy::List(_) | KTy::Map(..) => {
                let f = self.bytes_size_fn(ty, t);
                vec![add_to(S_ACC, expr(ExprKind::Call(f, vec![Arg::Value(load(p, ty))]), KTy::I64))]
            }
            _ => unreachable!("the checker refuses `to_bytes` of a value holding a {ty:?}"),
        }
    }

    /// `__kestrel_bytes_size_N(x: T) -> i64`: the size of `x`'s bytes, without the header.
    pub(crate) fn bytes_size_fn(&mut self, ty: KTy, t: &Tables) -> FuncIdx {
        if let Some(&f) = self.bytes_size.get(&ty) {
            return f;
        }
        let idx = self.reserve();
        self.bytes_size.insert(ty, idx);
        let x = Place::local(S_X);
        let mut body = vec![Stmt::Let(S_ACC, int(0, KTy::I64))];
        let at = local(S_I);
        match ty {
            KTy::Array(_) | KTy::List(_) => {
                let elem = elem_of(ty, t);
                body.push(add_to(S_ACC, int(4, KTy::I64)));
                match fixed_size(elem, t) {
                    Some(n) => body.push(add_to(S_ACC, wrapping(BinOp::MulWrap, len(&x), int(n, KTy::I64)))),
                    // The loop never changes the value, so `i` stays below its length.
                    None => {
                        let item = with(&x, Proj::Index { at, check: IndexCheck::Known });
                        let each = self.size_stmts(item, elem, t);
                        body.push(Stmt::ForRange { var: S_I, start: int(0, KTy::I64), end: len(&x), body: each });
                    }
                }
            }
            KTy::Map(..) => {
                let (k, v) = key_value(ty, t);
                body.push(add_to(S_ACC, int(4, KTy::I64)));
                match (fixed_size(k, t), fixed_size(v, t)) {
                    (Some(a), Some(b)) => body.push(add_to(S_ACC, wrapping(BinOp::MulWrap, len(&x), int(a + b, KTy::I64)))),
                    _ => {
                        let mut entry = self.size_stmts(with(&x, Proj::MapKey { at: at.clone() }), k, t);
                        entry.extend(self.size_stmts(with(&x, Proj::MapValue { at: at.clone(), checked: false }), v, t));
                        body.push(each_live(&x, at, entry, S_I));
                    }
                }
            }
            _ => body.extend(self.size_stmts(x, ty, t)),
        }
        body.push(Stmt::Return(Some(local(S_ACC))));
        let locals = vec![Local { name: "x".into(), ty, kind: param_kind(ty) }, number("acc"), number("i")];
        self.funcs[(idx.0 - self.base) as usize] = helper(format!("__kestrel_bytes_size_{}", idx.0), vec![S_X], KTy::I64, locals, body, false);
        idx
    }

    /// Statements that write the value at `p`, of type `ty`, at the local `W_AT` of a write function
    /// into its list (of type `list`) and move `W_AT` past it.
    fn write_stmts(&mut self, p: Place, ty: KTy, list: KTy, t: &Tables) -> Vec<Stmt> {
        if let Some(n) = word_size(ty) {
            return put(ListOp::WriteBytes, load(p, ty), int(n, KTy::I64));
        }
        match ty {
            KTy::Str => {
                let size = wrapping(BinOp::AddWrap, int(4, KTy::I64), len(&p));
                put(ListOp::WriteBytes, load(p, ty), size)
            }
            KTy::Struct(s) => {
                let mut out = Vec::new();
                for (i, f) in t.structs[s.0 as usize].fields.iter().enumerate() {
                    out.extend(self.write_stmts(with(&p, Proj::Field(i as u32)), kty(f.ty), list, t));
                }
                out
            }
            KTy::Enum(e) => {
                let tag = expr(ExprKind::Cast(Box::new(expr(ExprKind::Tag(p.clone()), KTy::U8))), KTy::I64);
                let mut out = put(ListOp::WriteCount, tag, int(4, KTy::I64));
                for (v, variant) in t.enums[e.0 as usize].variants.iter().enumerate() {
                    let mut then_body = Vec::new();
                    for (i, f) in variant.fields.iter().enumerate() {
                        then_body.extend(self.write_stmts(with(&p, Proj::Variant(v as u32, i as u32)), kty(f.ty), list, t));
                    }
                    if !then_body.is_empty() {
                        out.push(Stmt::If { cond: tag_is(&p, v as u32), then_body, else_body: Vec::new() });
                    }
                }
                out
            }
            KTy::Optional(e) => {
                let mut then_body = put(ListOp::WriteBytes, int(1, KTy::U8), int(1, KTy::I64));
                then_body.extend(self.write_stmts(with(&p, Proj::Payload), kty(t.elems[e.0 as usize]), list, t));
                let else_body = put(ListOp::WriteBytes, int(0, KTy::U8), int(1, KTy::I64));
                vec![Stmt::If { cond: has_value(&p), then_body, else_body }]
            }
            KTy::Array(_) | KTy::List(_) | KTy::Map(..) => {
                let f = self.bytes_write_fn(ty, list, t);
                let args = vec![Arg::Ref(Place::local(W_DST)), Arg::Value(local(W_AT)), Arg::Value(load(p, ty))];
                vec![Stmt::Assign(Place::local(W_AT), expr(ExprKind::Call(f, args), KTy::I64))]
            }
            _ => unreachable!("the checker refuses `to_bytes` of a value holding a {ty:?}"),
        }
    }

    /// `__kestrel_bytes_write_N(var dst: list[u8], at: i64, x: T) -> i64`: writes `x`'s bytes into
    /// `dst` from byte `at` (its length already covers them) and gives the byte after them.
    pub(crate) fn bytes_write_fn(&mut self, ty: KTy, list: KTy, t: &Tables) -> FuncIdx {
        if let Some(&f) = self.bytes_write.get(&ty) {
            return f;
        }
        let idx = self.reserve();
        self.bytes_write.insert(ty, idx);
        let x = Place::local(W_X);
        let at = local(W_I);
        let mut body = Vec::new();
        match ty {
            KTy::Array(_) | KTy::List(_) => {
                body.extend(put(ListOp::WriteCount, len(&x), int(4, KTy::I64)));
                // The loop never changes the value, so `i` stays below its length.
                let item = with(&x, Proj::Index { at, check: IndexCheck::Known });
                let each = self.write_stmts(item, elem_of(ty, t), list, t);
                body.push(Stmt::ForRange { var: W_I, start: int(0, KTy::I64), end: len(&x), body: each });
            }
            KTy::Map(..) => {
                let (k, v) = key_value(ty, t);
                body.extend(put(ListOp::WriteCount, len(&x), int(4, KTy::I64)));
                let mut entry = self.write_stmts(with(&x, Proj::MapKey { at: at.clone() }), k, list, t);
                entry.extend(self.write_stmts(with(&x, Proj::MapValue { at: at.clone(), checked: false }), v, list, t));
                body.push(each_live(&x, at, entry, W_I));
            }
            _ => body.extend(self.write_stmts(x, ty, list, t)),
        }
        body.push(Stmt::Return(Some(local(W_AT))));
        let locals = vec![
            Local { name: "dst".into(), ty: list, kind: LocalKind::RefParam },
            number("at"),
            Local { name: "x".into(), ty, kind: param_kind(ty) },
            number("i"),
        ];
        let name = format!("__kestrel_bytes_write_{}", idx.0);
        self.funcs[(idx.0 - self.base) as usize] = helper(name, vec![W_DST, W_AT, W_X], KTy::I64, locals, body, true);
        idx
    }

    /// `__kestrel_to_bytes_N(x: T) -> list[u8]`: `x`'s bytes with the header, in one new list (of
    /// type `list`) in the current arena whose room is its length.
    pub(crate) fn bytes_fn(&mut self, ty: KTy, list: KTy, t: &Tables) -> FuncIdx {
        if let Some(&f) = self.to_bytes.get(&ty) {
            return f;
        }
        let idx = self.reserve();
        self.to_bytes.insert(ty, idx);
        let size = self.bytes_size_fn(ty, t);
        let write = self.bytes_write_fn(ty, list, t);
        let x = || Arg::Value(load(Place::local(T_X), ty));
        let out = Place::local(T_OUT);
        let header = |at: i64, value: Expr| Stmt::List(ListOp::WriteBytes, out.clone(), vec![int(at, KTy::I64), value]);
        let body = vec![
            Stmt::Let(T_N, wrapping(BinOp::AddWrap, int(HEADER, KTy::I64), expr(ExprKind::Call(size, vec![x()]), KTy::I64))),
            Stmt::Let(T_OUT, expr(ExprKind::ListNew, list)),
            Stmt::List(ListOp::Reserve, out.clone(), vec![local(T_N)]),
            // The room is exactly `n`, and every byte below is written before anything reads it.
            Stmt::SetLen(out.clone(), local(T_N)),
            header(0, int(MAGIC, KTy::I32)),
            header(4, int(VERSION, KTy::U8)),
            header(5, int(fingerprint(ty, t), KTy::I64)),
            Stmt::Call(write, vec![Arg::Ref(out.clone()), Arg::Value(int(HEADER, KTy::I64)), x()], 0),
            Stmt::Return(Some(load(out, list))),
        ];
        let locals = vec![
            Local { name: "x".into(), ty, kind: param_kind(ty) },
            number("n"),
            Local { name: "out".into(), ty: list, kind: LocalKind::Addressed },
        ];
        self.funcs[(idx.0 - self.base) as usize] = helper(format!("__kestrel_to_bytes_{}", idx.0), vec![T_X], list, locals, body, false);
        idx
    }
}

/// `for i from 0 to m.used { if live(i) { entry } }` over the map at `m`, where `at` loads `i`.
fn each_live(m: &Place, at: Expr, entry: Vec<Stmt>, i: LocalIdx) -> Stmt {
    let live = expr(ExprKind::Map(MapOp::Live, m.clone(), vec![at]), KTy::Bool);
    Stmt::ForRange {
        var: i,
        start: int(0, KTy::I64),
        end: expr(ExprKind::Map(MapOp::Used, m.clone(), Vec::new()), KTy::I64),
        body: vec![Stmt::If { cond: live, then_body: entry, else_body: Vec::new() }],
    }
}

impl FnLowerer<'_> {
    /// `to_bytes(x)`, whose result has type `list` (`list[u8]`): a call to the type's `to_bytes`
    /// function.
    pub(crate) fn bytes_call(&mut self, arg: &ast::Expr, list: KTy) -> ExprKind {
        let ty = kty(self.ty(arg.id));
        let x = self.expr(arg);
        ExprKind::Call(self.helpers.bytes_fn(ty, list, self.t), vec![Arg::Value(x)])
    }
}
