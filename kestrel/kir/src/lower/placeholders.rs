//! Types that hold a placeholder: a generic declaration (`Stack`, which means `Stack[T]`), an
//! instance with a placeholder in its arguments (`Stack[T]` in other generic code), and an
//! element type holding one (`list[T]`). They exist in the checker's tables, so KIR keeps an
//! entry for each to line indexes up (`StructIdx == StructId`, `EnumIdx == EnumId`, and
//! `ElemIdx == ElemId`), but the entry is an empty placeholder and nothing may name it: only
//! copies are lowered, under their substitution, and their types are concrete.
//!
//! `check_concrete` makes sure of that at the end of `lower` (P16): `kty` panics on a bare
//! `Ty::Param`, but `kty` of `list[T]` is a `KTy::List` of a placeholder element, so a type that
//! skipped substitution would otherwise reach codegen.

use kestrel_check::Tables;

use super::kty;
use crate::ir::*;
use crate::layout::{EnumLayout, StructLayout};
use crate::visit::{walk, Node};

const UNSUBSTITUTED: &str = "a generic type reached KIR unsubstituted; this is a compiler bug";

/// The layout of a struct that holds a placeholder: no fields.
pub(super) fn struct_placeholder(name: String) -> StructLayout {
    StructLayout { name, fields: Vec::new(), size: 0, align: 1 }
}

/// The layout of an enum that holds a placeholder: no variants.
pub(super) fn enum_placeholder(name: String) -> EnumLayout {
    EnumLayout { name, variants: Vec::new(), size: 0, align: 1 }
}

/// `Module::elems`: each element type's KIR type, and a placeholder (`I64`, a plain number)
/// for one that holds a placeholder.
pub(super) fn elems(t: &Tables) -> Vec<KTy> {
    t.elems.iter().map(|&e| if t.is_concrete(e) { kty(e) } else { KTy::I64 }).collect()
}

/// Which structs, enums and element types are concrete, by index.
struct Concrete {
    structs: Vec<bool>,
    enums: Vec<bool>,
    elems: Vec<bool>,
}

impl Concrete {
    fn new(t: &Tables) -> Self {
        Concrete {
            structs: t.structs.iter().map(|s| s.concrete).collect(),
            enums: t.enums.iter().map(|e| e.concrete).collect(),
            elems: t.elems.iter().map(|&e| t.is_concrete(e)).collect(),
        }
    }

    /// Panics if `ty` names a placeholder layout or element.
    fn check(&self, ty: KTy) {
        let ok = match ty {
            KTy::Struct(s) => self.structs[s.0 as usize],
            KTy::Enum(e) => self.enums[e.0 as usize],
            KTy::Array(e) | KTy::List(e) | KTy::Optional(e) => self.elems[e.0 as usize],
            KTy::Map(k, v) => self.elems[k.0 as usize] && self.elems[v.0 as usize],
            KTy::Bool
            | KTy::U8
            | KTy::I32
            | KTy::I64
            | KTy::F32
            | KTy::F64
            | KTy::F32X4
            | KTy::I32X4
            | KTy::Str
            | KTy::Arena
            | KTy::Gpu
            | KTy::Fn
            | KTy::Iface => true,
        };
        if !ok {
            panic!("{UNSUBSTITUTED}");
        }
    }

    fn check_sig(&self, sig: &KSig) {
        sig.params.iter().for_each(|&(ty, _)| self.check(ty));
        sig.ret.into_iter().for_each(|ty| self.check(ty));
    }
}

/// Panics if any function's local, result or expression type, any struct or variant built, any
/// signature of an indirect or virtual call, any global, or any printed type names a type that
/// holds a placeholder (P16).
pub(super) fn check_concrete(m: &Module, t: &Tables) {
    let c = Concrete::new(t);
    for f in &m.funcs {
        f.locals.iter().for_each(|l| c.check(l.ty));
        f.ret.into_iter().for_each(|ty| c.check(ty));
        walk(&f.body, &mut |n| match n {
            Node::Expr(e) => {
                c.check(e.ty);
                match &e.kind {
                    ExprKind::CallIndirect { sig, .. } | ExprKind::CallVirtual { sig, .. } => c.check_sig(sig),
                    ExprKind::StructLit(s, _) => c.check(KTy::Struct(*s)),
                    ExprKind::VariantLit(en, ..) => c.check(KTy::Enum(*en)),
                    _ => {}
                }
            }
            Node::Stmt(Stmt::CallIndirect { sig, .. } | Stmt::CallVirtual { sig, .. }) => c.check_sig(sig),
            Node::Stmt(_) | Node::Place(..) => {}
        });
    }
    m.globals.iter().for_each(|&ty| c.check(ty));
    m.print_fns.keys().for_each(|&ty| c.check(ty));
}

#[cfg(test)]
mod tests {
    use kestrel_check::{ParamId, Ty};
    use kestrel_syntax::{parse, LineIndex};

    use super::*;

    const SRC: &str = "struct Stack[T] {\n    items: list[T] = []\n}\nmain() {\n    let s: Stack[int] = Stack(items: [4])\n    print(s.items.len)\n}\n";

    /// The program's module and tables, and the declaration `Stack`'s index.
    fn lowered() -> (Module, Tables, u32) {
        let (p, _) = parse(SRC);
        let t = kestrel_check::check(&p).0.expect("checks");
        let m = super::super::lower(&p, &t, &LineIndex::new(SRC));
        let decl = t.structs.iter().position(|s| s.name == "Stack").expect("the declaration") as u32;
        (m, t, decl)
    }

    #[test]
    #[should_panic(expected = "a generic type reached KIR unsubstituted; this is a compiler bug")]
    fn a_local_of_a_placeholder_layout_panics() {
        let (mut m, t, decl) = lowered();
        m.funcs[0].locals.push(Local { name: "x".into(), ty: KTy::Struct(StructIdx(decl)), kind: LocalKind::Value });
        check_concrete(&m, &t);
    }

    #[test]
    #[should_panic(expected = "a generic type reached KIR unsubstituted; this is a compiler bug")]
    fn a_struct_built_of_a_placeholder_layout_panics() {
        let (mut m, t, decl) = lowered();
        let lit = Expr { kind: ExprKind::StructLit(StructIdx(decl), Vec::new()), ty: KTy::I32, line: 0 };
        m.funcs[0].body.push(Stmt::Return(Some(lit)));
        check_concrete(&m, &t);
    }

    #[test]
    #[should_panic(expected = "a generic type reached KIR unsubstituted; this is a compiler bug")]
    fn a_list_of_a_placeholder_panics() {
        let (mut m, t, _) = lowered();
        let elem = t.elems.iter().position(|&e| !t.is_concrete(e)).expect("`T` is an element type") as u32;
        m.funcs[0].ret = Some(KTy::List(ElemIdx(elem)));
        check_concrete(&m, &t);
    }

    #[test]
    #[should_panic(expected = "a generic type reached KIR unsubstituted; this is a compiler bug")]
    fn kty_of_a_placeholder_panics() {
        kty(Ty::Param(ParamId(0)));
    }

    #[test]
    fn the_lowered_module_passes() {
        let (m, t, _) = lowered();
        check_concrete(&m, &t);
    }
}
