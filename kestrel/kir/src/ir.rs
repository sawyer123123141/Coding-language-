//! Kestrel's typed, structured middle layer. Loops stay loops; no
//! Cranelift (or any backend) types appear here.

use std::collections::HashMap;

use crate::layout::{EnumLayout, StructLayout};
pub use crate::ir_expr::*;
pub use crate::ir_stmt::*;
pub use crate::ops::{ListOp, MapOp, MathOp, StrOp};

/// Bits of a KIR line (`Expr::line`, `Stmt::SetLine`) that hold the line within its file; the
/// bits above hold the file's index in `Module::files`. Bit 31 stays clear.
pub const LINE_BITS: u32 = 21;
/// The file index that means "no name" (a file past the 1,023rd).
pub const NO_FILE: u32 = 1023;

/// A source position as KIR keeps it: the line within its file, and the file's index. A line
/// past 2,097,151 is clamped; a file index past 1,022 becomes `NO_FILE`.
pub fn pack_line(file: usize, line: u32) -> u32 {
    let file = u32::try_from(file).unwrap_or(NO_FILE).min(NO_FILE);
    (file << LINE_BITS) | line.min((1 << LINE_BITS) - 1)
}

/// The line and the file index of a packed KIR line (see `pack_line`).
pub fn unpack_line(loc: u32) -> (u32, u32) {
    (loc & ((1 << LINE_BITS) - 1), loc >> LINE_BITS)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum KTy {
    Bool,
    U8,
    I32,
    I64,
    F32,
    F64,
    /// A struct, laid out by `Module::structs`. Lives in memory, never in a register.
    Struct(StructIdx),
    /// An enum, laid out by `Module::enums`: a u8 tag, then the variant's fields.
    Enum(EnumIdx),
    /// A string: a pointer to UTF-8 bytes at offset 0, its length (i64) at 8.
    Str,
    /// `[T]`: a pointer to the elements at 0, the length (i64) at 8.
    Array(ElemIdx),
    /// `list[T]`: pointer, length, capacity (i64s), then the arena it grows in.
    List(ElemIdx),
    /// A pointer to a runtime arena (null until created).
    Arena,
    /// A GPU object's id (an `i64`; 0 is none), owned like an arena.
    Gpu,
    /// `T?`: a u8 tag (0 = `none`) at offset 0, then the value.
    Optional(ElemIdx),
    /// `map[K, V]`: a 64-byte header (see `MapLayout`) over entries and an
    /// index table in the map's arena.
    Map(ElemIdx, ElemIdx),
    /// A function value, whatever its signature: the code's address at offset 0 and the
    /// environment (an `i64`; 0 when there is none) at 8. The code takes the environment first,
    /// then the parameters (see `KSig`).
    Fn,
}

/// What an indirect call needs to know about the function it calls: each parameter's type and
/// whether it is passed by pointer (`var`, or an aggregate: the same rule as `LocalKind::RefParam`
/// for a parameter), and the result.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct KSig {
    pub params: Vec<(KTy, bool)>,
    pub ret: Option<KTy>,
}

impl KTy {
    pub fn is_aggregate(self) -> bool {
        matches!(
            self,
            KTy::Struct(_)
                | KTy::Enum(_)
                | KTy::Str
                | KTy::Array(_)
                | KTy::List(_)
                | KTy::Map(..)
                | KTy::Arena
                | KTy::Gpu
                | KTy::Optional(_)
                | KTy::Fn
        )
    }

    pub fn is_float(self) -> bool {
        matches!(self, KTy::F32 | KTy::F64)
    }

    pub fn is_int(self) -> bool {
        matches!(self, KTy::U8 | KTy::I32 | KTy::I64)
    }

    pub fn is_signed(self) -> bool {
        matches!(self, KTy::I32 | KTy::I64)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct FuncIdx(pub u32);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct LocalIdx(pub u32);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct StructIdx(pub u32);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct EnumIdx(pub u32);

/// An element type of arrays and lists, in `Module::elems`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ElemIdx(pub u32);

/// A zero-filled global in `Module::globals`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct GlobalIdx(pub u32);

#[derive(Clone, Debug, PartialEq)]
pub struct Module {
    pub funcs: Vec<Func>,
    pub structs: Vec<StructLayout>,
    pub enums: Vec<EnumLayout>,
    pub elems: Vec<KTy>,
    /// The generated function that prints each array or list type printed.
    pub print_fns: HashMap<KTy, FuncIdx>,
    /// `par for` bodies, each called through `Stmt::ParFor`.
    pub par_bodies: Vec<FuncIdx>,
    /// Zero-filled global memory, by `GlobalIdx`. Empty without heap constants; otherwise
    /// global 0 is the "built" flag (`Bool`), global 1 the constants' arena (`Arena`), and
    /// one more follows for each heap constant that is used.
    pub globals: Vec<KTy>,
    /// `__kestrel_constants`, which builds the heap constants once (it can run again and does
    /// nothing). `main` calls it first; a host that calls other functions runs it itself.
    pub init: Option<FuncIdx>,
    /// `None` for a library (see `kestrel_check::check_with`).
    pub main: Option<FuncIdx>,
    /// The source files' names (base names, like `geom.kes`), by the file index in each line
    /// (see `pack_line`). Empty when the compiler wasn't given names: runtime errors then give
    /// only the line.
    pub files: Vec<String>,
}

/// A memory location: a local, or a path of fields and elements inside one.
#[derive(Clone, Debug, PartialEq)]
pub struct Place {
    pub root: LocalIdx,
    /// Steps from the root inward.
    pub path: Vec<Proj>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Proj {
    /// A struct field.
    Field(u32),
    /// Field `.1` of variant `.0` of an enum (valid while that variant is stored).
    Variant(u32, u32),
    /// The value inside an optional (valid while it isn't `none`).
    Payload,
    /// An element of an array or list. The index is any integer type. It is
    /// bounds-checked unless the compiler proved it in bounds (a `for ... in`
    /// loop's own index, when the body can't move the elements).
    Index { at: Expr, checked: bool },
    /// The key of entry `at` of a map (its index in the entries, not a key).
    MapKey { at: Expr },
    /// The value of entry `at` of a map. Checked, it panics unless the
    /// entry is still there (the loop body may have replaced the map).
    MapValue { at: Expr, checked: bool },
}

impl Proj {
    /// The index expression inside an element or entry projection.
    pub fn at(&self) -> Option<&Expr> {
        match self {
            Proj::Index { at, .. } | Proj::MapKey { at } | Proj::MapValue { at, .. } => Some(at),
            Proj::Field(_) | Proj::Variant(..) | Proj::Payload => None,
        }
    }

    pub fn at_mut(&mut self) -> Option<&mut Expr> {
        match self {
            Proj::Index { at, .. } | Proj::MapKey { at } | Proj::MapValue { at, .. } => Some(at),
            Proj::Field(_) | Proj::Variant(..) | Proj::Payload => None,
        }
    }
}

impl Place {
    pub fn local(root: LocalIdx) -> Self {
        Place { root, path: Vec::new() }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Func {
    pub name: String,
    pub params: Vec<LocalIdx>,
    pub ret: Option<KTy>,
    pub locals: Vec<Local>,
    pub body: Vec<Stmt>,
    pub is_pure: bool,
    pub io: bool,
    pub mutates_params: bool,
    /// `pub` and in the entry file: a host (the Godot add-on) may call it.
    pub exported: bool,
    /// For a `native` function: the symbol it imports (see `native_symbol`).
    /// It has no body; Rust implements it with Kestrel's calling convention.
    pub native: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Local {
    pub name: String,
    pub ty: KTy,
    pub kind: LocalKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LocalKind {
    /// Lives in a register.
    Value,
    /// Needs a memory slot: lowering (and inlining) assigns it to a struct, or to a local passed
    /// as a `var` argument somewhere. `scalarize` may turn it into `Value` (a number whose `var`
    /// argument inlining removed) or `Unused` (a plain struct, split into one `Value` local per
    /// field).
    Addressed,
    /// Holds a pointer to the caller's value: a `var` parameter, or any
    /// aggregate parameter (read-only ones are passed by pointer too). Also
    /// a `for ... in` element (`Stmt::Bind`) or a heap constant (`Stmt::BindGlobal`).
    RefParam,
    /// Nothing refers to it any more: a struct `scalarize` split into one local per field.
    Unused,
}
