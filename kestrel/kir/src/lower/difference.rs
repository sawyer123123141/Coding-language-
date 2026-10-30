//! `first_difference(a, b)`: where two values of one type first differ, or `none` (the I/O round's
//! spec, decision 3). The values are walked in the order `to_bytes` writes them (see `bytes`) by a
//! generated function per type, `__kestrel_diff_N(a: T, b: T, var out: string) -> bool`, which
//! gives true when they differ and then has written to `out` where, and both values:
//!
//! - numbers, bools and strings compare whole, floats by their bits (`Hash(Bits)`), so `-0.0` and
//!   `0.0` differ and two NaNs with the same bits are equal. The text is `path: a and b`, each as
//!   `print` shows it inside a value (a string quoted); two floats that print alike show their
//!   bits after (`103.25 and 103.25 (bits 0x42CE8000 and 0x42CE8001)`). A `vec4` or `ivec4` is
//!   its lanes, `.x` to `.w`;
//! - a struct is its fields in order (`.ball.x`), an enum its variant (`Alive and Dead`) and then
//!   that variant's fields (`.state.since`), an optional whether it has a value (`none and a
//!   value`) and then its payload, under the same path;
//! - a list or array is its length (`.len: 3 and 4`, nothing deeper) and then its items
//!   (`[2]`), a map each of `a`'s live keys in insertion order (`["ana"]: 3 and missing` when `b`
//!   lacks it), then each key only `b` has (`["bo"]: missing and 4`), then, when the entries are
//!   the same, their order (`: the same keys in another order`). These go through their own
//!   functions, so types that contain themselves through a collection work, as `checksum`'s do.
//!
//! The path is built from the inside out, only once a difference is found: a collection's
//! function writes its part (`[2].p1.up: true and false`), and each caller puts its own part in
//! front. So equal values allocate nothing. `__kestrel_first_difference_N(a: T, b: T) -> string?`
//! drops the leading `.` or `: ` (a top-level scalar gives `1.5 and 1.25`) and wraps the text. The
//! text is built in the current arena. The checker refuses values holding arenas, GPU objects,
//! interface values or function values (`kestrel_check::snapshot`).

use kestrel_check::Tables;
use kestrel_syntax::ast;

use super::helpers::{elem_of, expr, has_value, tag_is, with, Helpers};
use super::map_helpers::{each_entry_of, key_value};
use super::{kty, FnLowerer};
use crate::ir::*;

// The locals of a diff function: the two values, the text it writes, two loop indexes, the two
// values' texts (a float's or an enum variant's name) and, in a map's, `b`'s value for `a`'s key.
const A: LocalIdx = LocalIdx(0);
const B: LocalIdx = LocalIdx(1);
const OUT: LocalIdx = LocalIdx(2);
const I: LocalIdx = LocalIdx(3);
const J: LocalIdx = LocalIdx(4);
const TA: LocalIdx = LocalIdx(5);
const TB: LocalIdx = LocalIdx(6);
const FOUND: LocalIdx = LocalIdx(7);

/// A vector's lanes, as a path names them.
const LANES: [&str; 4] = ["x", "y", "z", "w"];

fn text(s: &str) -> PrintArg {
    PrintArg::Str(s.to_string())
}

fn value(e: Expr) -> PrintArg {
    PrintArg::Value(e)
}

fn load(p: Place, ty: KTy) -> Expr {
    expr(ExprKind::Load(p), ty)
}

fn local(l: LocalIdx, ty: KTy) -> Expr {
    load(Place::local(l), ty)
}

fn int(n: i64) -> Expr {
    expr(ExprKind::Int(n), KTy::I64)
}

fn boolean(b: bool) -> Expr {
    expr(ExprKind::Bool(b), KTy::Bool)
}

fn binary(op: BinOp, a: Expr, b: Expr, ty: KTy) -> Expr {
    expr(ExprKind::Binary(op, Box::new(a), Box::new(b)), ty)
}

fn not(e: Expr) -> Expr {
    expr(ExprKind::Unary(UnOp::Not, Box::new(e)), KTy::Bool)
}

fn when(cond: Expr, then_body: Vec<Stmt>) -> Stmt {
    Stmt::If { cond, then_body, else_body: Vec::new() }
}

fn format(pieces: Vec<PrintArg>) -> Expr {
    expr(ExprKind::Format(pieces), KTy::Str)
}

/// The length (or a map's live count) of the value at `p`.
fn len(p: &Place) -> Expr {
    expr(ExprKind::Len(p.clone()), KTy::I64)
}

/// The value at `p`, of type `ty`, as `print` shows it inside a value: a string quoted.
fn shown(p: Place, ty: KTy) -> Vec<PrintArg> {
    let v = value(load(p, ty));
    if ty == KTy::Str {
        vec![text("\""), v, text("\"")]
    } else {
        vec![v]
    }
}

/// `path` with `more` after it.
fn then(path: &[PrintArg], more: impl IntoIterator<Item = PrintArg>) -> Vec<PrintArg> {
    let mut out = path.to_vec();
    out.extend(more);
    out
}

/// `path: a and b`
fn both(path: &[PrintArg], a: Vec<PrintArg>, b: Vec<PrintArg>) -> Vec<PrintArg> {
    let mut out = then(path, [text(": ")]);
    out.extend(a);
    out.push(text(" and "));
    out.extend(b);
    out
}

/// `out = pieces; return true`
fn report(pieces: Vec<PrintArg>) -> Vec<Stmt> {
    vec![Stmt::Assign(Place::local(OUT), format(pieces)), Stmt::Return(Some(boolean(true)))]
}

/// The two floats (`float` or `f64`) at `a` and `b`, when their bits differ: as `print` shows
/// them, and their bits too when that is the same text.
fn float_diff(a: Place, b: Place, ty: KTy, path: &[PrintArg]) -> Stmt {
    let bits = |p: &Place| expr(ExprKind::Hash(HashOp::Bits, vec![load(p.clone(), ty)]), KTy::I64);
    let digits = if ty == KTy::F32 { 8 } else { 16 };
    let hex = |p: &Place| value(expr(ExprKind::Str(StrOp::Hex, vec![bits(p), int(digits)]), KTy::Str));
    let plain = both(path, vec![value(local(TA, KTy::Str))], vec![value(local(TB, KTy::Str))]);
    let mut with_bits = plain.clone();
    with_bits.extend([text(" (bits "), hex(&a), text(" and "), hex(&b), text(")")]);
    let alike = binary(BinOp::Eq, local(TA, KTy::Str), local(TB, KTy::Str), KTy::Bool);
    let body = vec![
        Stmt::Assign(Place::local(TA), format(vec![value(load(a.clone(), ty))])),
        Stmt::Assign(Place::local(TB), format(vec![value(load(b.clone(), ty))])),
        Stmt::If { cond: alike, then_body: report(with_bits), else_body: report(plain) },
    ];
    when(binary(BinOp::Ne, bits(&a), bits(&b), KTy::Bool), body)
}

/// A parameter's kind: an aggregate is passed by pointer.
fn param_kind(ty: KTy) -> LocalKind {
    if ty.is_aggregate() {
        LocalKind::RefParam
    } else {
        LocalKind::Value
    }
}

impl Helpers {
    /// Statements that, when the values at `a` and `b` (of type `ty`) differ, write where and how
    /// to `OUT` (after `path`, this function's part of the path so far) and return true.
    fn diff_stmts(&mut self, a: Place, b: Place, ty: KTy, path: &[PrintArg], t: &Tables) -> Vec<Stmt> {
        match ty {
            KTy::Bool | KTy::U8 | KTy::I32 | KTy::I64 | KTy::Str => {
                let differ = binary(BinOp::Ne, load(a.clone(), ty), load(b.clone(), ty), KTy::Bool);
                vec![when(differ, report(both(path, shown(a, ty), shown(b, ty))))]
            }
            KTy::F32 | KTy::F64 => vec![float_diff(a, b, ty, path)],
            KTy::F32X4 | KTy::I32X4 => {
                let mut out = Vec::new();
                for (l, name) in LANES.iter().enumerate() {
                    let lane = |p: &Place| with(p, Proj::Lane(l as u8));
                    out.extend(self.diff_stmts(lane(&a), lane(&b), ty.lane(), &then(path, [text(&format!(".{name}"))]), t));
                }
                out
            }
            KTy::Struct(s) => {
                let mut out = Vec::new();
                for (i, f) in t.structs[s.0 as usize].fields.iter().enumerate() {
                    let field = |p: &Place| with(p, Proj::Field(i as u32));
                    out.extend(self.diff_stmts(field(&a), field(&b), kty(f.ty), &then(path, [text(&format!(".{}", f.name))]), t));
                }
                out
            }
            KTy::Enum(e) => {
                let variants = &t.enums[e.0 as usize].variants;
                // Another variant: the two variants' names.
                let mut names = Vec::new();
                for (v, variant) in variants.iter().enumerate() {
                    for (l, p) in [(TA, &a), (TB, &b)] {
                        let name = expr(ExprKind::StrLit(variant.name.clone()), KTy::Str);
                        names.push(when(tag_is(p, v as u32), vec![Stmt::Assign(Place::local(l), name)]));
                    }
                }
                names.extend(report(both(path, vec![value(local(TA, KTy::Str))], vec![value(local(TB, KTy::Str))])));
                let tag = |p: &Place| expr(ExprKind::Tag(p.clone()), KTy::U8);
                let mut out = vec![when(binary(BinOp::Ne, tag(&a), tag(&b), KTy::Bool), names)];
                // The same variant: its fields.
                for (v, variant) in variants.iter().enumerate() {
                    let mut fields = Vec::new();
                    for (i, f) in variant.fields.iter().enumerate() {
                        let field = |p: &Place| with(p, Proj::Variant(v as u32, i as u32));
                        fields.extend(self.diff_stmts(field(&a), field(&b), kty(f.ty), &then(path, [text(&format!(".{}", f.name))]), t));
                    }
                    if !fields.is_empty() {
                        out.push(when(tag_is(&a, v as u32), fields));
                    }
                }
                out
            }
            KTy::Optional(el) => {
                let mut some = vec![when(not(has_value(&b)), report(then(path, [text(": a value and none")])))];
                some.extend(self.diff_stmts(with(&a, Proj::Payload), with(&b, Proj::Payload), kty(t.elems[el.0 as usize]), path, t));
                let none = vec![when(has_value(&b), report(then(path, [text(": none and a value")])))];
                vec![Stmt::If { cond: has_value(&a), then_body: some, else_body: none }]
            }
            KTy::Array(_) | KTy::List(_) | KTy::Map(..) => {
                let f = self.diff_fn(ty, t);
                let args = vec![Arg::Value(load(a, ty)), Arg::Value(load(b, ty)), Arg::Ref(Place::local(OUT))];
                let mut found = Vec::new();
                if !path.is_empty() {
                    // This function's part goes in front of the collection's.
                    found.push(Stmt::Assign(Place::local(OUT), format(then(path, [value(local(OUT, KTy::Str))]))));
                }
                found.push(Stmt::Return(Some(boolean(true))));
                vec![when(expr(ExprKind::Call(f, args), KTy::Bool), found)]
            }
            _ => unreachable!("the checker refuses `first_difference` of a value holding a {ty:?}"),
        }
    }

    /// A list's or array's walk: its length, then its items.
    fn diff_items(&mut self, ty: KTy, t: &Tables) -> Vec<Stmt> {
        let (a, b) = (Place::local(A), Place::local(B));
        let lengths = both(&[text(".len")], vec![value(len(&a))], vec![value(len(&b))]);
        let at = local(I, KTy::I64);
        // The lengths are equal and nothing changes either value, so `i` is below both.
        let item = |p: &Place| with(p, Proj::Index { at: at.clone(), check: IndexCheck::Known });
        let path = [text("["), value(at.clone()), text("]")];
        let each = self.diff_stmts(item(&a), item(&b), elem_of(ty, t), &path, t);
        vec![
            when(binary(BinOp::Ne, len(&a), len(&b), KTy::Bool), report(lengths)),
            Stmt::ForRange { var: I, start: int(0), end: len(&a), body: each },
        ]
    }

    /// A map's walk: `a`'s keys in order, then the keys only `b` has, then the order of the keys.
    fn diff_entries(&mut self, ty: KTy, t: &Tables) -> Vec<Stmt> {
        let (a, b) = (Place::local(A), Place::local(B));
        let (k, v) = key_value(ty, t);
        let KTy::Map(_, v_elem) = ty else { unreachable!("{ty:?} isn't a map") };
        // A value with no partner is printed whole.
        self.ensure_print(v, t);
        let mut body = Vec::new();
        body.push(each_entry_of(a.clone(), I, |key, val| {
            let path = then(&[text("[")], shown(key.clone(), k).into_iter().chain([text("]")]));
            let found = Place::local(FOUND);
            let get = expr(ExprKind::Map(MapOp::Get, b.clone(), vec![load(key, k)]), KTy::Optional(v_elem));
            let missing = report(both(&path, shown(val.clone(), v), vec![text("missing")]));
            let same = self.diff_stmts(val, with(&found, Proj::Payload), v, &path, t);
            vec![Stmt::Let(FOUND, get), Stmt::If { cond: has_value(&found), then_body: same, else_body: missing }]
        }));
        body.push(each_entry_of(b.clone(), I, |key, val| {
            let path = then(&[text("[")], shown(key.clone(), k).into_iter().chain([text("]")]));
            let has = expr(ExprKind::Map(MapOp::Has, a.clone(), vec![load(key, k)]), KTy::Bool);
            vec![when(not(has), report(both(&path, vec![text("missing")], shown(val, v))))]
        }));
        // The same keys and values: `b`'s `j`-th live key against `a`'s. Both have as many live
        // keys, so `j` stays below `b`'s entries.
        let key_a = with(&a, Proj::MapKey { at: local(I, KTy::I64) });
        let j = local(J, KTy::I64);
        let step = Stmt::Assign(Place::local(J), binary(BinOp::AddWrap, j.clone(), int(1), KTy::I64));
        let live = expr(ExprKind::Map(MapOp::Live, b.clone(), vec![j.clone()]), KTy::Bool);
        let key_b = with(&b, Proj::MapKey { at: j });
        let order = report(vec![text(": the same keys in another order")]);
        body.push(Stmt::Let(J, int(0)));
        body.push(each_entry_of(a, I, |_, _| {
            vec![
                Stmt::While { cond: not(live), body: vec![step.clone()] },
                when(binary(BinOp::Ne, load(key_a, k), load(key_b, k), KTy::Bool), order),
                step,
            ]
        }));
        body
    }

    /// `__kestrel_diff_N(a: T, b: T, var out: string) -> bool`: whether `a` and `b` differ, and if
    /// so, where and how in `out` (this function's part of the path first).
    pub(crate) fn diff_fn(&mut self, ty: KTy, t: &Tables) -> FuncIdx {
        if let Some(&f) = self.difference.get(&ty) {
            return f;
        }
        let idx = self.reserve();
        self.difference.insert(ty, idx);
        let empty = || expr(ExprKind::StrLit(String::new()), KTy::Str);
        let mut body = vec![Stmt::Let(TA, empty()), Stmt::Let(TB, empty())];
        match ty {
            KTy::Array(_) | KTy::List(_) => body.extend(self.diff_items(ty, t)),
            KTy::Map(..) => body.extend(self.diff_entries(ty, t)),
            _ => body.extend(self.diff_stmts(Place::local(A), Place::local(B), ty, &[], t)),
        }
        body.push(Stmt::Return(Some(boolean(false))));
        let number = |name: &str| Local { name: name.into(), ty: KTy::I64, kind: LocalKind::Value };
        let string = |name: &str| Local { name: name.into(), ty: KTy::Str, kind: LocalKind::Addressed };
        let mut locals = vec![
            Local { name: "a".into(), ty, kind: param_kind(ty) },
            Local { name: "b".into(), ty, kind: param_kind(ty) },
            Local { name: "out".into(), ty: KTy::Str, kind: LocalKind::RefParam },
            number("i"),
            number("j"),
            string("ta"),
            string("tb"),
        ];
        if let KTy::Map(_, v) = ty {
            locals.push(Local { name: "found".into(), ty: KTy::Optional(v), kind: LocalKind::Addressed });
        }
        self.funcs[(idx.0 - self.base) as usize] = Func {
            name: format!("__kestrel_diff_{}", idx.0),
            params: vec![A, B, OUT],
            ret: Some(KTy::Bool),
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

    /// `__kestrel_first_difference_N(a: T, b: T) -> string?` (`result` is `string?`): the diff
    /// function's text without its leading `.` or `: `, or `none`.
    pub(crate) fn first_difference_fn(&mut self, ty: KTy, result: KTy, t: &Tables) -> FuncIdx {
        if let Some(&f) = self.first_difference.get(&ty) {
            return f;
        }
        let idx = self.reserve();
        self.first_difference.insert(ty, idx);
        let diff = self.diff_fn(ty, t);
        let out = Place::local(OUT);
        let s = || local(OUT, KTy::Str);
        let starts = |prefix: &str| expr(ExprKind::Str(StrOp::Starts, vec![s(), expr(ExprKind::StrLit(prefix.into()), KTy::Str)]), KTy::Bool);
        // The text starts with the prefix, so it is at least `n` bytes long.
        let cut = |n: i64| {
            let rest = binary(BinOp::SubWrap, len(&out), int(n), KTy::I64);
            Stmt::Assign(out.clone(), expr(ExprKind::Str(StrOp::Piece, vec![s(), int(n), rest]), KTy::Str))
        };
        let args = vec![Arg::Value(local(A, ty)), Arg::Value(local(B, ty)), Arg::Ref(out.clone())];
        let found = vec![
            Stmt::If { cond: starts("."), then_body: vec![cut(1)], else_body: vec![when(starts(": "), vec![cut(2)])] },
            Stmt::Return(Some(expr(ExprKind::Some(Box::new(s())), result))),
        ];
        let body = vec![
            Stmt::Let(OUT, expr(ExprKind::StrLit(String::new()), KTy::Str)),
            when(expr(ExprKind::Call(diff, args), KTy::Bool), found),
            Stmt::Return(Some(expr(ExprKind::NoneLit, result))),
        ];
        let locals = vec![
            Local { name: "a".into(), ty, kind: param_kind(ty) },
            Local { name: "b".into(), ty, kind: param_kind(ty) },
            Local { name: "out".into(), ty: KTy::Str, kind: LocalKind::Addressed },
        ];
        self.funcs[(idx.0 - self.base) as usize] = Func {
            name: format!("__kestrel_first_difference_{}", idx.0),
            params: vec![A, B],
            ret: Some(result),
            locals,
            body,
            is_pure: true,
            io: false,
            mutates_params: false,
            exported: false,
            lines: (0, 0),
            native: None,
        };
        idx
    }
}

impl FnLowerer<'_> {
    /// `first_difference(a, b)`, whose value has type `result` (`string?`): a call to the type's
    /// `first_difference` function.
    pub(crate) fn difference_call(&mut self, args: &[ast::Arg], result: KTy) -> ExprKind {
        let ty = kty(self.ty(args[0].value.id));
        let (a, b) = (self.expr(&args[0].value), self.expr(&args[1].value));
        ExprKind::Call(self.helpers.first_difference_fn(ty, result, self.t), vec![Arg::Value(a), Arg::Value(b)])
    }
}

#[cfg(test)]
#[path = "difference_test.rs"]
mod tests;
