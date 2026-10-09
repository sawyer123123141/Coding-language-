use kestrel_check::Tables;

use super::kty;
use crate::ir::{FieldLayout, KTy, StructLayout};

/// Lays out every struct: fields in declaration order at natural alignment.
/// The checker guarantees structs don't contain themselves.
pub(crate) fn layouts(t: &Tables) -> Vec<StructLayout> {
    let mut done: Vec<Option<StructLayout>> = vec![None; t.structs.len()];
    for i in 0..t.structs.len() {
        layout_of(t, i, &mut done);
    }
    done.into_iter().map(|l| l.expect("every struct is laid out")).collect()
}

fn layout_of(t: &Tables, i: usize, done: &mut [Option<StructLayout>]) -> (u32, u32) {
    if let Some(l) = &done[i] {
        return (l.size, l.align);
    }
    let s = &t.structs[i];
    let (mut offset, mut align) = (0u32, 1u32);
    let mut fields = Vec::with_capacity(s.fields.len());
    for f in &s.fields {
        let ty = kty(f.ty);
        let (size, field_align) = match ty {
            KTy::Struct(inner) => layout_of(t, inner.0 as usize, done),
            KTy::Bool | KTy::U8 => (1, 1),
            KTy::I32 | KTy::F32 => (4, 4),
            KTy::I64 | KTy::F64 => (8, 8),
        };
        offset = offset.next_multiple_of(field_align);
        fields.push(FieldLayout { name: f.name.clone(), ty, offset });
        offset += size;
        align = align.max(field_align);
    }
    let size = offset.next_multiple_of(align);
    done[i] = Some(StructLayout { name: s.name.clone(), fields, size, align });
    (size, align)
}
