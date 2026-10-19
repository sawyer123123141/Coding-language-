//! Sizes, alignments and offsets of KIR types in memory.

use crate::ir::*;

/// The tag (u8, offset 0) says which variant is stored; each variant's
/// fields start at the same payload offset, so they share memory.
#[derive(Clone, Debug, PartialEq)]
pub struct EnumLayout {
    pub name: String,
    pub variants: Vec<VariantLayout>,
    pub size: u32,
    pub align: u32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct VariantLayout {
    pub name: String,
    /// Offsets are from the start of the enum value.
    pub fields: Vec<FieldLayout>,
}

/// Fields in declaration order at natural alignment (like C).
#[derive(Clone, Debug, PartialEq)]
pub struct StructLayout {
    pub name: String,
    pub fields: Vec<FieldLayout>,
    pub size: u32,
    pub align: u32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct FieldLayout {
    pub name: String,
    pub ty: KTy,
    pub offset: u32,
}

impl Module {
    pub fn layout(&self, s: StructIdx) -> &StructLayout {
        &self.structs[s.0 as usize]
    }

    pub fn enum_layout(&self, e: EnumIdx) -> &EnumLayout {
        &self.enums[e.0 as usize]
    }

    pub fn elem(&self, e: ElemIdx) -> KTy {
        self.elems[e.0 as usize]
    }

    /// Where the value inside an optional of `T` starts.
    pub fn payload_offset(&self, e: ElemIdx) -> u32 {
        let (size, align) = self.size_align(self.elem(e));
        optional_layout(size, align).0
    }
}

/// An optional of a value with this size and alignment: the payload's
/// offset, then the optional's size and alignment.
pub fn optional_layout(size: u32, align: u32) -> (u32, u32, u32) {
    let align = align.max(1);
    (align, (align + size).next_multiple_of(align), align)
}

impl Module {
    /// Size and alignment in bytes.
    pub fn size_align(&self, ty: KTy) -> (u32, u32) {
        match ty {
            KTy::Bool | KTy::U8 => (1, 1),
            KTy::I32 | KTy::F32 => (4, 4),
            KTy::I64 | KTy::F64 | KTy::Arena | KTy::Gpu => (8, 8),
            KTy::Str | KTy::Array(_) | KTy::Fn | KTy::Iface => (16, 8),
            KTy::List(_) => (32, 8),
            KTy::Map(..) => (64, 8),
            KTy::Optional(e) => {
                let (size, align) = self.size_align(self.elem(e));
                let (_, size, align) = optional_layout(size, align);
                (size, align)
            }
            KTy::Struct(s) => {
                let l = self.layout(s);
                (l.size, l.align)
            }
            KTy::Enum(e) => {
                let l = self.enum_layout(e);
                (l.size, l.align)
            }
        }
    }
}

/// Where a map's entries keep their parts. The header is `entries` (0),
/// `count` (8), `used` (16), `cap` (24), `slots` (32), `slot_count` (40),
/// `locks` (48) and `arena` (56). An entry is the key's hash (u32, 0), a
/// live flag (u8, 4), the key (8), then the value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MapLayout {
    /// 0, 1, 2: integers (or bools) of 1, 4, 8 bytes; 3: a string.
    pub key_kind: i32,
    pub val_off: u32,
    pub entry_size: u32,
    pub entry_align: u32,
}

impl Module {
    pub fn map_layout(&self, key: ElemIdx, value: ElemIdx) -> MapLayout {
        let (key_size, _) = self.size_align(self.elem(key));
        let key_kind = match self.elem(key) {
            KTy::Bool | KTy::U8 => 0,
            KTy::I32 => 1,
            KTy::I64 => 2,
            KTy::Str => 3,
            other => unreachable!("{other:?} isn't a map key type"),
        };
        let (size, align) = self.size_align(self.elem(value));
        let entry_align = align.max(8);
        let val_off = (8 + key_size).next_multiple_of(align.max(1));
        let entry_size = (val_off + size).next_multiple_of(entry_align);
        MapLayout { key_kind, val_off, entry_size, entry_align }
    }
}
