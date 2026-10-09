use std::collections::HashMap;

use kestrel_syntax::ast::{BindId, ExprId, FnDecl, Item, Program};
use kestrel_syntax::Span;

use crate::ty::Ty;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct LocalId(pub u32);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct FnId(pub u32);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ConstId(pub u32);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct StructId(pub u32);

/// The built-in vector types are the first structs, before any user struct.
pub const VEC2: StructId = StructId(0);
pub const VEC3: StructId = StructId(1);
pub const BUILTIN_STRUCTS: u32 = 2;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct EnumId(pub u32);

/// What a name or call expression refers to.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Res {
    Local(LocalId),
    Const(ConstId),
    Fn(FnId),
    Print,
    Convert(Ty),
    /// `Point(...)`: builds a struct.
    Construct(StructId),
    /// `Block.Air` or `Block.Stone(...)`: builds an enum value.
    Variant(EnumId, u32),
}

#[derive(Clone, Debug)]
pub struct LocalInfo {
    pub name: String,
    pub ty: Ty,
    pub mutable: bool,
    pub is_param: bool,
    pub is_var_param: bool,
    /// Passed as a `var` argument somewhere, so it needs a memory slot.
    pub addressed: bool,
    pub owner: FnId,
    pub span: Span,
    /// A `match` binding aliases the matched variable: changes go to it.
    pub alias: Option<LocalId>,
}

#[derive(Clone, Debug)]
pub struct ParamSig {
    pub name: String,
    pub ty: Ty,
    pub is_var: bool,
}

#[derive(Clone, Debug)]
pub struct FnInfo {
    pub name: String,
    /// Index into `Program::items`.
    pub item: usize,
    /// For a method: its index in its struct's or enum's `methods`.
    pub method: Option<usize>,
    pub is_pure: bool,
    pub params: Vec<ParamSig>,
    pub ret: Ty,
    pub span: Span,
    /// Every local owned by this function, parameters first in order.
    pub locals: Vec<LocalId>,
    pub io: bool,
    pub mutates_params: bool,
}

impl FnInfo {
    /// The function's (or method's) declaration in the AST.
    pub fn decl<'p>(&self, program: &'p Program) -> &'p FnDecl {
        match (&program.items[self.item], self.method) {
            (Item::Fn(f), None) => f,
            (Item::Struct(s), Some(m)) => &s.methods[m],
            (Item::Enum(e), Some(m)) => &e.methods[m],
            _ => unreachable!("FnInfo points at a function or a method"),
        }
    }
}

#[derive(Clone, Debug)]
pub struct StructInfo {
    pub name: String,
    /// Index into `Program::items`; `usize::MAX` for built-in structs.
    pub item: usize,
    pub span: Span,
    pub fields: Vec<FieldInfo>,
    pub methods: HashMap<String, FnId>,
}

#[derive(Clone, Debug)]
pub struct EnumInfo {
    pub name: String,
    pub item: usize,
    pub span: Span,
    pub variants: Vec<VariantInfo>,
    pub methods: HashMap<String, FnId>,
}

#[derive(Clone, Debug)]
pub struct VariantInfo {
    pub name: String,
    pub span: Span,
    pub fields: Vec<FieldInfo>,
}

#[derive(Clone, Debug)]
pub struct FieldInfo {
    pub name: String,
    pub ty: Ty,
    pub span: Span,
    pub has_default: bool,
}

#[derive(Clone, Debug)]
pub struct ConstInfo {
    pub name: String,
    pub ty: Ty,
    pub item: usize,
}

/// Everything the checker learned, keyed by AST ids.
#[derive(Debug, Default)]
pub struct Tables {
    pub expr_ty: Vec<Ty>,
    pub coerce: HashMap<ExprId, Ty>,
    pub res: HashMap<ExprId, Res>,
    pub bind_local: HashMap<BindId, LocalId>,
    /// For a call: entry `i` is the index (into the call's args) of the
    /// argument bound to parameter `i`.
    pub call_args: HashMap<ExprId, Vec<usize>>,
    pub locals: Vec<LocalInfo>,
    pub fns: Vec<FnInfo>,
    pub consts: Vec<ConstInfo>,
    pub structs: Vec<StructInfo>,
    pub struct_by_name: HashMap<String, StructId>,
    pub enums: Vec<EnumInfo>,
    pub enum_by_name: HashMap<String, EnumId>,
    /// For a `match`: per arm, the variant it matches, or `None` for `else`.
    pub match_arms: HashMap<ExprId, Vec<Option<u32>>>,
    /// For a field access `a.b`: the index of `b` in its struct.
    pub field_index: HashMap<ExprId, u32>,
    /// For a struct or variant constructor: per field, the argument index,
    /// or `None` for the default.
    pub ctor_args: HashMap<ExprId, Vec<Option<usize>>>,
    /// For a swizzle like `v.xz`: the components it picks, in order.
    pub swizzles: HashMap<ExprId, Vec<u32>>,
    pub fn_by_name: HashMap<String, FnId>,
    pub const_by_name: HashMap<String, ConstId>,
    pub main: Option<FnId>,
}

impl Tables {
    pub fn ty(&self, e: ExprId) -> Ty {
        self.expr_ty[e.0 as usize]
    }

    pub fn local(&self, id: LocalId) -> &LocalInfo {
        &self.locals[id.0 as usize]
    }

    pub fn func(&self, id: FnId) -> &FnInfo {
        &self.fns[id.0 as usize]
    }

    pub fn strukt(&self, id: StructId) -> &StructInfo {
        &self.structs[id.0 as usize]
    }

    /// The number of components when `ty` is `vec2` or `vec3`.
    pub fn vec_dims(&self, ty: Ty) -> Option<u32> {
        match ty {
            Ty::Struct(VEC2) => Some(2),
            Ty::Struct(VEC3) => Some(3),
            _ => None,
        }
    }

    pub fn enm(&self, id: EnumId) -> &EnumInfo {
        &self.enums[id.0 as usize]
    }

    /// How a type reads in messages: struct types by name.
    pub fn ty_name(&self, ty: Ty) -> String {
        match ty {
            Ty::Struct(id) => self.strukt(id).name.clone(),
            Ty::Enum(id) => self.enm(id).name.clone(),
            Ty::Variant(id, v) => format!("{}.{}", self.enm(id).name, self.enm(id).variants[v as usize].name),
            other => other.name().to_string(),
        }
    }
}
