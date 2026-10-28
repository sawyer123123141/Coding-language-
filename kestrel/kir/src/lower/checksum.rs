//! `checksum(x)`: the value's contents folded in a fixed order (the syntax spec's "Built-in
//! functions"; the runtime's `checksum` has the steps). A number, bool, `vec4`, `ivec4` or string
//! is one `Hash(Mix)`; anything else goes through a generated function per type,
//! `__kestrel_checksum_N(h: i64, x: T) -> i64`, which folds `x` into `h`:
//!
//! - a struct is its fields in order (`vec2`, `vec3`, `ivec2` and `ivec3` are structs: their
//!   lanes in order), an enum its variant number then that variant's fields, an optional 0, or 1
//!   then its payload;
//! - an array or list is its length, then its first `length` items (never its capacity or what a
//!   shrunk list keeps past its length), and a map its live count, then each live key and value
//!   in insertion order. These always go through their own function, so types that contain
//!   themselves through a collection work.
//!
//! Padding is never read. The checker refuses values holding arenas, GPU objects, interface
//! values or function values (`kestrel_check::snapshot`). Nothing here allocates or fails.

use kestrel_check::Tables;
use kestrel_syntax::ast;

use super::helpers::{elem_of, expr, has_value, tag_is, with, Helpers};
use super::{kty, FnLowerer};
use crate::ir::*;

/// The fold's starting value (the runtime's `checksum::START`).
const START: i64 = 0x243F_6A88_85A3_08D3;

/// A checksum function's locals: the fold it starts from, the value, the fold so far and a loop
/// index.
const H: LocalIdx = LocalIdx(0);
const X: LocalIdx = LocalIdx(1);
const ACC: LocalIdx = LocalIdx(2);
const I: LocalIdx = LocalIdx(3);

/// Whether one `Hash(Mix)` takes a value of type `ty` whole.
fn is_word(ty: KTy) -> bool {
    matches!(ty, KTy::Bool | KTy::U8 | KTy::I32 | KTy::I64 | KTy::F32 | KTy::F64 | KTy::F32X4 | KTy::I32X4 | KTy::Str)
}

fn mix(h: Expr, x: Expr) -> Expr {
    expr(ExprKind::Hash(HashOp::Mix, vec![h, x]), KTy::I64)
}

fn acc() -> Expr {
    expr(ExprKind::Load(Place::local(ACC)), KTy::I64)
}

/// `acc = mix(acc, x)`
fn mix_in(x: Expr) -> Stmt {
    Stmt::Assign(Place::local(ACC), mix(acc(), x))
}

fn word(n: i64) -> Expr {
    expr(ExprKind::Int(n), KTy::I64)
}

impl Helpers {
    /// Statements that fold the value at `p`, of type `ty`, into the local `ACC`.
    fn fold(&mut self, p: Place, ty: KTy, t: &Tables) -> Vec<Stmt> {
        match ty {
            _ if is_word(ty) => vec![mix_in(expr(ExprKind::Load(p), ty))],
            KTy::Struct(s) => {
                let mut out = Vec::new();
                for (i, f) in t.structs[s.0 as usize].fields.iter().enumerate() {
                    out.extend(self.fold(with(&p, Proj::Field(i as u32)), kty(f.ty), t));
                }
                out
            }
            KTy::Enum(e) => {
                let mut out = vec![mix_in(expr(ExprKind::Tag(p.clone()), KTy::U8))];
                for (v, variant) in t.enums[e.0 as usize].variants.iter().enumerate() {
                    let mut then_body = Vec::new();
                    for (i, f) in variant.fields.iter().enumerate() {
                        then_body.extend(self.fold(with(&p, Proj::Variant(v as u32, i as u32)), kty(f.ty), t));
                    }
                    if !then_body.is_empty() {
                        out.push(Stmt::If { cond: tag_is(&p, v as u32), then_body, else_body: Vec::new() });
                    }
                }
                out
            }
            KTy::Optional(e) => {
                let mut then_body = vec![mix_in(word(1))];
                then_body.extend(self.fold(with(&p, Proj::Payload), kty(t.elems[e.0 as usize]), t));
                vec![Stmt::If { cond: has_value(&p), then_body, else_body: vec![mix_in(word(0))] }]
            }
            KTy::Array(_) | KTy::List(_) | KTy::Map(..) => {
                let f = self.checksum_fn(ty, t);
                let call = ExprKind::Call(f, vec![Arg::Value(acc()), Arg::Value(expr(ExprKind::Load(p), ty))]);
                vec![Stmt::Assign(Place::local(ACC), expr(call, KTy::I64))]
            }
            KTy::Arena | KTy::Gpu | KTy::Fn | KTy::Iface => unreachable!("the checker refuses `checksum` of a value holding a {ty:?}"),
            _ => unreachable!("{ty:?} is a word"),
        }
    }

    /// `__kestrel_checksum_N(h, x)`: `x`, of type `ty` (not a word), folded into `h`.
    pub(crate) fn checksum_fn(&mut self, ty: KTy, t: &Tables) -> FuncIdx {
        if let Some(&f) = self.checksum.get(&ty) {
            return f;
        }
        let idx = self.reserve();
        self.checksum.insert(ty, idx);
        let x = Place::local(X);
        let at = expr(ExprKind::Load(Place::local(I)), KTy::I64);
        let mut body = vec![Stmt::Let(ACC, expr(ExprKind::Load(Place::local(H)), KTy::I64))];
        match ty {
            // The loops never change the value, so `i` stays below its length.
            KTy::Array(_) | KTy::List(_) => {
                let item = with(&x, Proj::Index { at, check: IndexCheck::Known });
                body.push(mix_in(expr(ExprKind::Len(x.clone()), KTy::I64)));
                body.push(Stmt::ForRange {
                    var: I,
                    start: word(0),
                    end: expr(ExprKind::Len(x), KTy::I64),
                    body: self.fold(item, elem_of(ty, t), t),
                });
            }
            KTy::Map(k, v) => {
                let (k, v) = (kty(t.elems[k.0 as usize]), kty(t.elems[v.0 as usize]));
                let live = expr(ExprKind::Map(MapOp::Live, x.clone(), vec![at.clone()]), KTy::Bool);
                let mut entry = self.fold(with(&x, Proj::MapKey { at: at.clone() }), k, t);
                entry.extend(self.fold(with(&x, Proj::MapValue { at, checked: false }), v, t));
                body.push(mix_in(expr(ExprKind::Len(x.clone()), KTy::I64)));
                body.push(Stmt::ForRange {
                    var: I,
                    start: word(0),
                    end: expr(ExprKind::Map(MapOp::Used, x, Vec::new()), KTy::I64),
                    body: vec![Stmt::If { cond: live, then_body: entry, else_body: Vec::new() }],
                });
            }
            _ => body.extend(self.fold(x, ty, t)),
        }
        body.push(Stmt::Return(Some(acc())));
        let value = |name: &str| Local { name: name.into(), ty: KTy::I64, kind: LocalKind::Value };
        self.funcs[(idx.0 - self.base) as usize] = Func {
            name: format!("__kestrel_checksum_{}", idx.0),
            params: vec![H, X],
            ret: Some(KTy::I64),
            locals: vec![value("h"), Local { name: "x".into(), ty, kind: LocalKind::RefParam }, value("acc"), value("i")],
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
    /// `checksum(x)`: the finished fold of `x` from the start value.
    pub(crate) fn checksum(&mut self, arg: &ast::Expr, line: u32) -> ExprKind {
        let ty = kty(self.ty(arg.id));
        let x = self.expr(arg);
        let start = Expr { kind: ExprKind::Int(START), ty: KTy::I64, line };
        let folded = if is_word(ty) {
            ExprKind::Hash(HashOp::Mix, vec![start, x])
        } else {
            ExprKind::Call(self.helpers.checksum_fn(ty, self.t), vec![Arg::Value(start), Arg::Value(x)])
        };
        ExprKind::Hash(HashOp::Finish, vec![Expr { kind: folded, ty: KTy::I64, line }])
    }
}
