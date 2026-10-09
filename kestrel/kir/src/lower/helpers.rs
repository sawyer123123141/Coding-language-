//! Functions generated during lowering, one per type that needs them: deep
//! copies and printing of arrays and lists. Structs and enums are handled
//! inline; collections always go through a call, so types that contain
//! themselves through a collection (`struct Node { kids: [Node] }`) work.

use std::collections::HashMap;

use kestrel_check::Tables;

use super::kty;
use crate::ir::*;

/// Whether a value of `ty` holds arrays or lists (directly or in fields).
/// Strings are immutable, so copies share them and they don't count.
pub(crate) fn has_heap(ty: KTy, t: &Tables) -> bool {
    match ty {
        KTy::Array(_) | KTy::List(_) => true,
        KTy::Struct(s) => t.structs[s.0 as usize].fields.iter().any(|f| has_heap(kty(f.ty), t)),
        KTy::Enum(e) => t.enums[e.0 as usize].variants.iter().flat_map(|v| &v.fields).any(|f| has_heap(kty(f.ty), t)),
        _ => false,
    }
}

fn elem_of(ty: KTy, t: &Tables) -> KTy {
    match ty {
        KTy::Array(e) | KTy::List(e) => kty(t.elems[e.0 as usize]),
        other => unreachable!("{other:?} has no elements"),
    }
}

fn expr(kind: ExprKind, ty: KTy) -> Expr {
    Expr { kind, ty, line: 0 }
}

fn with(place: &Place, proj: Proj) -> Place {
    let mut p = place.clone();
    p.path.push(proj);
    p
}

pub(crate) struct Helpers {
    /// The index of the first helper: helpers follow the user's functions.
    base: u32,
    pub(crate) funcs: Vec<Func>,
    copy: HashMap<KTy, FuncIdx>,
    pub(crate) print: HashMap<KTy, FuncIdx>,
}

impl Helpers {
    pub(crate) fn new(base: u32) -> Self {
        Helpers { base, funcs: Vec::new(), copy: HashMap::new(), print: HashMap::new() }
    }

    /// Statements that give the value at `place`, just copied byte for
    /// byte, its own copy of every array and list inside it.
    pub(crate) fn fixup(&mut self, place: Place, ty: KTy, t: &Tables) -> Vec<Stmt> {
        let mut out = Vec::new();
        match ty {
            KTy::Array(_) | KTy::List(_) => out.push(Stmt::Call(self.copy_fn(ty, t), vec![Arg::Ref(place)])),
            KTy::Struct(s) => {
                for (i, f) in t.structs[s.0 as usize].fields.iter().enumerate() {
                    if has_heap(kty(f.ty), t) {
                        out.extend(self.fixup(with(&place, Proj::Field(i as u32)), kty(f.ty), t));
                    }
                }
            }
            KTy::Enum(e) => {
                for (v, variant) in t.enums[e.0 as usize].variants.iter().enumerate() {
                    let mut then_body = Vec::new();
                    for (i, f) in variant.fields.iter().enumerate() {
                        if has_heap(kty(f.ty), t) {
                            let p = with(&place, Proj::Variant(v as u32, i as u32));
                            then_body.extend(self.fixup(p, kty(f.ty), t));
                        }
                    }
                    if !then_body.is_empty() {
                        let tag = expr(ExprKind::Tag(place.clone()), KTy::U8);
                        let v = expr(ExprKind::Int(v as i64), KTy::U8);
                        let cond = expr(ExprKind::Binary(BinOp::Eq, Box::new(tag), Box::new(v)), KTy::Bool);
                        out.push(Stmt::If { cond, then_body, else_body: Vec::new() });
                    }
                }
            }
            _ => {}
        }
        out
    }

    /// Makes sure every array or list type inside `ty` has a printer.
    pub(crate) fn ensure_print(&mut self, ty: KTy, t: &Tables) {
        match ty {
            KTy::Array(_) | KTy::List(_) => {
                self.print_fn(ty, t);
            }
            KTy::Struct(s) => {
                for f in &t.structs[s.0 as usize].fields {
                    self.ensure_print(kty(f.ty), t);
                }
            }
            KTy::Enum(e) => {
                for f in t.enums[e.0 as usize].variants.iter().flat_map(|v| &v.fields) {
                    self.ensure_print(kty(f.ty), t);
                }
            }
            _ => {}
        }
    }

    /// Reserves a slot for a helper, so a type can refer to its own helper.
    fn reserve(&mut self) -> FuncIdx {
        let idx = FuncIdx(self.base + self.funcs.len() as u32);
        let placeholder = Func {
            name: String::new(),
            params: Vec::new(),
            ret: None,
            locals: Vec::new(),
            body: Vec::new(),
            is_pure: true,
            io: false,
            mutates_params: false,
        };
        self.funcs.push(placeholder);
        idx
    }

    /// Locals of a helper: the collection (by pointer) and a loop index.
    fn locals(ty: KTy) -> Vec<Local> {
        vec![
            Local { name: "xs".into(), ty, kind: LocalKind::RefParam },
            Local { name: "i".into(), ty: KTy::I64, kind: LocalKind::Value },
        ]
    }

    /// `for i from 0 to xs.len { body(xs[i]) }`
    fn each(body: impl FnOnce(Place) -> Vec<Stmt>) -> Stmt {
        let (xs, i) = (Place::local(LocalIdx(0)), LocalIdx(1));
        // The body never moves the elements, so `i < xs.len` holds throughout.
        let at = with(&xs, Proj::Index { at: expr(ExprKind::Load(Place::local(i)), KTy::I64), checked: false });
        Stmt::ForRange {
            var: i,
            start: expr(ExprKind::Int(0), KTy::I64),
            end: expr(ExprKind::Len(xs), KTy::I64),
            body: body(at),
        }
    }

    /// `copy(var xs)`: a fresh buffer for `xs`, then each element's own copies.
    fn copy_fn(&mut self, ty: KTy, t: &Tables) -> FuncIdx {
        if let Some(&f) = self.copy.get(&ty) {
            return f;
        }
        let idx = self.reserve();
        self.copy.insert(ty, idx);
        let elem = elem_of(ty, t);
        let mut body = vec![Stmt::Dup(Place::local(LocalIdx(0)))];
        if has_heap(elem, t) {
            body.push(Self::each(|at| self.fixup(at, elem, t)));
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
        };
        idx
    }

    /// `print(xs)` without a newline: `[1, 2, 3]`, strings quoted.
    fn print_fn(&mut self, ty: KTy, t: &Tables) -> FuncIdx {
        if let Some(&f) = self.print.get(&ty) {
            return f;
        }
        let idx = self.reserve();
        self.print.insert(ty, idx);
        let elem = elem_of(ty, t);
        self.ensure_print(elem, t);
        let text = |s: &str| PrintArg::Str(s.to_string());
        let loop_ = Self::each(|at| {
            let i = expr(ExprKind::Load(Place::local(LocalIdx(1))), KTy::I64);
            let first = expr(ExprKind::Binary(BinOp::Gt, Box::new(i), Box::new(expr(ExprKind::Int(0), KTy::I64))), KTy::Bool);
            let item = PrintArg::Value(expr(ExprKind::Load(at), elem));
            let piece = if elem == KTy::Str { vec![text("\""), item, text("\"")] } else { vec![item] };
            vec![
                Stmt::If { cond: first, then_body: vec![Stmt::Write(vec![text(", ")])], else_body: Vec::new() },
                Stmt::Write(piece),
            ]
        });
        let body = vec![Stmt::Write(vec![text("[")]), loop_, Stmt::Write(vec![text("]")])];
        self.funcs[(idx.0 - self.base) as usize] = Func {
            name: format!("__kestrel_print_{}", idx.0),
            params: vec![LocalIdx(0)],
            ret: None,
            locals: Self::locals(ty),
            body,
            is_pure: false,
            io: true,
            mutates_params: false,
        };
        idx
    }
}
