//! The generated printers for arrays and lists (see `helpers`).

use kestrel_check::Tables;

use super::helpers::{elem_of, expr, Helpers};
use super::kty;
use crate::ir::*;

impl Helpers {
    /// Makes sure every array or list type inside `ty` has a printer.
    pub(crate) fn ensure_print(&mut self, ty: KTy, t: &Tables) {
        match ty {
            KTy::Array(_) | KTy::List(_) => {
                self.print_fn(ty, t);
            }
            KTy::Map(..) => {
                self.map_print_fn(ty, t);
            }
            KTy::Optional(e) => self.ensure_print(kty(t.elems[e.0 as usize]), t),
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
            exported: false,
            lines: (0, 0),
            native: None,
        };
        idx
    }
}
