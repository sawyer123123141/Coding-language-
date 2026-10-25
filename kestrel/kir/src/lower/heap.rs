//! Which KIR types hold heap memory or own an arena or a GPU object.

use kestrel_check::Tables;

use super::kty;
use crate::ir::*;

/// Whether a value of `ty` holds arrays, lists, maps or strings (directly or
/// in fields). Strings may live in an arena (interpolation builds them
/// there), so copies copy their bytes, like arrays.
pub(crate) fn has_heap(ty: KTy, t: &Tables) -> bool {
    has_heap_below(ty, t, &mut Vec::new())
}

/// `has_heap`, skipping the types in `seen`: the checker refuses a type that
/// contains itself, but a layout walk that looped would crash the compiler.
fn has_heap_below(ty: KTy, t: &Tables, seen: &mut Vec<KTy>) -> bool {
    match ty {
        // A function value's environment (a closure's captures) is heap memory, and so is an
        // interface value's boxed concrete value.
        KTy::Str | KTy::Array(_) | KTy::List(_) | KTy::Map(..) | KTy::Fn | KTy::Iface => true,
        KTy::Optional(e) => has_heap_below(kty(t.elems[e.0 as usize]), t, seen),
        KTy::Struct(_) | KTy::Enum(_) if seen.contains(&ty) => false,
        KTy::Struct(s) => {
            seen.push(ty);
            t.structs[s.0 as usize].fields.iter().any(|f| has_heap_below(kty(f.ty), t, seen))
        }
        KTy::Enum(e) => {
            seen.push(ty);
            t.enums[e.0 as usize].variants.iter().flat_map(|v| &v.fields).any(|f| has_heap_below(kty(f.ty), t, seen))
        }
        _ => false,
    }
}

/// Whether `ty` holds an `Arena` or a GPU object, so values of it move and get dropped.
pub(crate) fn owning(ty: KTy, t: &Tables) -> bool {
    match ty {
        KTy::Arena | KTy::Gpu => true,
        KTy::Struct(s) => t.owning_structs[s.0 as usize],
        KTy::Enum(e) => t.owning_enums[e.0 as usize],
        KTy::Array(e) | KTy::List(e) | KTy::Optional(e) => t.is_owning(t.elems[e.0 as usize]),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use kestrel_check::tables::ElemId;
    use kestrel_check::{FieldInfo, StructId, StructInfo, Tables, Ty};
    use kestrel_syntax::Span;

    use super::has_heap;
    use crate::ir::{KTy, StructIdx};

    fn field(name: &str, ty: Ty) -> FieldInfo {
        FieldInfo { name: name.into(), ty, span: Span { start: 0, end: 0 }, has_default: false, default_allocates: false }
    }

    fn strukt(name: &str, fields: Vec<FieldInfo>) -> StructInfo {
        let span = Span { start: 0, end: 0 };
        StructInfo { name: name.into(), item: 0, span, fields, methods: Default::default(), ifaces: Vec::new(), params: Vec::new(), instance: None, concrete: true }
    }

    /// The checker refuses a struct that holds itself, so this can't happen in a
    /// program; if a gap in the checker let one through, the walk must still end.
    #[test]
    fn a_struct_that_holds_itself_does_not_loop() {
        // Node { value: int, next: Node? }, and A { b: B?, s: string } with B { a: A? }.
        let t = Tables {
            elems: vec![Ty::Struct(StructId(0)), Ty::Struct(StructId(2)), Ty::Struct(StructId(1))],
            structs: vec![
                strukt("Node", vec![field("value", Ty::Int), field("next", Ty::Optional(ElemId(0)))]),
                strukt("A", vec![field("b", Ty::Optional(ElemId(1))), field("s", Ty::Str)]),
                strukt("B", vec![field("a", Ty::Optional(ElemId(2)))]),
            ],
            ..Tables::default()
        };
        assert!(!has_heap(KTy::Struct(StructIdx(0)), &t));
        assert!(has_heap(KTy::Struct(StructIdx(1)), &t));
        assert!(has_heap(KTy::Struct(StructIdx(2)), &t));
    }
}
