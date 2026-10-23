use kestrel_check::{FieldInfo, Tables};

use super::kty;
use crate::ir::KTy;
use crate::layout::{EnumLayout, FieldLayout, StructLayout, VariantLayout};

/// Lays out every struct and enum. Fields go in declaration order at
/// natural alignment; an enum is a u8 tag followed by a shared payload.
/// The checker guarantees no type contains itself.
pub(crate) fn layouts(t: &Tables) -> (Vec<StructLayout>, Vec<EnumLayout>) {
    let mut l = Layouts { t, structs: vec![None; t.structs.len()], enums: vec![None; t.enums.len()] };
    for i in 0..t.structs.len() {
        l.size_align(KTy::Struct(crate::ir::StructIdx(i as u32)));
    }
    for i in 0..t.enums.len() {
        l.size_align(KTy::Enum(crate::ir::EnumIdx(i as u32)));
    }
    let structs = l.structs.into_iter().map(|s| s.expect("every struct is laid out")).collect();
    let enums = l.enums.into_iter().map(|e| e.expect("every enum is laid out")).collect();
    (structs, enums)
}

/// The size and alignment of a value of `ty`.
pub(crate) fn size_align(t: &Tables, ty: KTy) -> (u32, u32) {
    Layouts { t, structs: vec![None; t.structs.len()], enums: vec![None; t.enums.len()] }.size_align(ty)
}

struct Layouts<'a> {
    t: &'a Tables,
    structs: Vec<Option<StructLayout>>,
    enums: Vec<Option<EnumLayout>>,
}

impl Layouts<'_> {
    fn size_align(&mut self, ty: KTy) -> (u32, u32) {
        match ty {
            KTy::Bool | KTy::U8 => (1, 1),
            KTy::I32 | KTy::F32 => (4, 4),
            KTy::I64 | KTy::F64 | KTy::Arena | KTy::Gpu => (8, 8),
            KTy::Str | KTy::Array(_) | KTy::Fn | KTy::Iface => (16, 8),
            KTy::F32X4 | KTy::I32X4 => (16, 16),
            KTy::List(_) => (32, 8),
            KTy::Map(..) => (64, 8),
            KTy::Optional(e) => {
                let (size, align) = self.size_align(super::kty(self.t.elems[e.0 as usize]));
                let (_, size, align) = crate::layout::optional_layout(size, align);
                (size, align)
            }
            KTy::Struct(s) => {
                let i = s.0 as usize;
                if self.structs[i].is_none() {
                    let t = self.t;
                    let info = &t.structs[i];
                    let (fields, end, align) = self.fields(&info.fields, 0);
                    let size = end.next_multiple_of(align);
                    self.structs[i] = Some(StructLayout { name: shown(&info.name), fields, size, align });
                }
                let l = self.structs[i].as_ref().expect("just laid out");
                (l.size, l.align)
            }
            KTy::Enum(e) => {
                let i = e.0 as usize;
                if self.enums[i].is_none() {
                    self.enums[i] = Some(self.enum_layout(i));
                }
                let l = self.enums[i].as_ref().expect("just laid out");
                (l.size, l.align)
            }
        }
    }

    /// Lays fields out one after another from `start`; returns them, the end
    /// offset, and the largest alignment.
    fn fields(&mut self, infos: &[FieldInfo], start: u32) -> (Vec<FieldLayout>, u32, u32) {
        let (mut offset, mut align) = (start, 1u32);
        let mut fields = Vec::with_capacity(infos.len());
        for f in infos {
            let ty = kty(f.ty);
            let (size, field_align) = self.size_align(ty);
            offset = offset.next_multiple_of(field_align);
            fields.push(FieldLayout { name: f.name.clone(), ty, offset });
            offset += size;
            align = align.max(field_align);
        }
        (fields, offset, align)
    }

    fn enum_layout(&mut self, i: usize) -> EnumLayout {
        let t = self.t;
        let info = &t.enums[i];
        // Every variant's payload starts at the same offset: just past the
        // tag, rounded up for the most-aligned field of any variant.
        let mut payload_align = 1;
        for v in &info.variants {
            for f in &v.fields {
                payload_align = payload_align.max(self.size_align(kty(f.ty)).1);
            }
        }
        let start = 1u32.next_multiple_of(payload_align);
        let mut end = 1;
        let mut variants = Vec::with_capacity(info.variants.len());
        for v in &info.variants {
            let (fields, variant_end, _) = self.fields(&v.fields, start);
            end = end.max(variant_end);
            variants.push(VariantLayout { name: v.name.clone(), fields });
        }
        EnumLayout { name: shown(&info.name), variants, size: end.next_multiple_of(payload_align), align: payload_align }
    }
}

/// A type's name as printed: without the module it came from (`geom.Point` prints as `Point`).
fn shown(name: &str) -> String {
    name.rsplit('.').next().unwrap_or(name).to_string()
}
