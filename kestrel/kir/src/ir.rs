//! Kestrel's typed, structured middle layer. Loops stay loops; no
//! Cranelift (or any backend) types appear here.

use std::collections::HashMap;

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
}

impl KTy {
    pub fn is_aggregate(self) -> bool {
        matches!(self, KTy::Struct(_) | KTy::Enum(_) | KTy::Str | KTy::Array(_) | KTy::List(_))
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

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct LocalIdx(pub u32);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct StructIdx(pub u32);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct EnumIdx(pub u32);

/// An element type of arrays and lists, in `Module::elems`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ElemIdx(pub u32);

#[derive(Clone, Debug, PartialEq)]
pub struct Module {
    pub funcs: Vec<Func>,
    pub structs: Vec<StructLayout>,
    pub enums: Vec<EnumLayout>,
    pub elems: Vec<KTy>,
    /// The generated function that prints each array or list type printed.
    pub print_fns: HashMap<KTy, FuncIdx>,
    pub main: FuncIdx,
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

    /// Size and alignment in bytes.
    pub fn size_align(&self, ty: KTy) -> (u32, u32) {
        match ty {
            KTy::Bool | KTy::U8 => (1, 1),
            KTy::I32 | KTy::F32 => (4, 4),
            KTy::I64 | KTy::F64 => (8, 8),
            KTy::Str | KTy::Array(_) => (16, 8),
            KTy::List(_) => (32, 8),
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
    /// An element of an array or list. The index is any integer type. It is
    /// bounds-checked unless the compiler proved it in bounds (a `for ... in`
    /// loop's own index, when the body can't move the elements).
    Index { at: Expr, checked: bool },
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
    /// Needs a memory slot: passed as a `var` argument somewhere, or a struct.
    Addressed,
    /// Holds a pointer to the caller's value: a `var` parameter, or any
    /// aggregate parameter (read-only ones are passed by pointer too). Also
    /// a `for ... in` element, set by `Stmt::Bind`.
    RefParam,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Stmt {
    Let(LocalIdx, Expr),
    Assign(Place, Expr),
    If { cond: Expr, then_body: Vec<Stmt>, else_body: Vec<Stmt> },
    While { cond: Expr, body: Vec<Stmt> },
    /// `for var from start to end`: `end` exclusive, step +1, both
    /// evaluated once before the first iteration.
    ForRange { var: LocalIdx, start: Expr, end: Expr, body: Vec<Stmt> },
    Break,
    Continue,
    Return(Option<Expr>),
    /// A call whose result (if any) is discarded.
    Call(FuncIdx, Vec<Arg>),
    Print(Vec<PrintArg>),
    /// Prints with no separators and no newline (used by generated printers).
    Write(Vec<PrintArg>),
    /// Appends a value to the list at a place, growing it if it's full.
    ListAdd(Place, Expr),
    /// Points a reference local at a place (a `for ... in` element, found
    /// once per iteration).
    Bind(LocalIdx, Place),
    /// Gives the array or list at a place its own copy of its elements, in
    /// the current arena (a list's capacity becomes its length).
    Dup(Place),
}

#[derive(Clone, Debug, PartialEq)]
pub enum PrintArg {
    Value(Expr),
    Str(String),
    /// An interpolated string: its parts print with nothing between them.
    Interp(Vec<PrintArg>),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Expr {
    pub kind: ExprKind,
    pub ty: KTy,
    /// Source line, for runtime error messages.
    pub line: u32,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ExprKind {
    Int(i64),
    Float(f64),
    Bool(bool),
    /// A string literal; the bytes live in read-only data.
    StrLit(String),
    /// `array[T](n)`: `n` zeroed elements in the current arena.
    ArrayNew(Box<Expr>),
    /// `[a, b]`: an array, or a list when `ty` is a list.
    ArrayLit(Vec<Expr>),
    /// `list[T]()`: empty, growing in the current arena.
    ListNew,
    /// The length (i64) of the array or list at a place.
    Len(Place),
    /// The value stored at a place (a scalar, or a whole struct).
    Load(Place),
    Unary(UnOp, Box<Expr>),
    Binary(BinOp, Box<Expr>, Box<Expr>),
    /// Short-circuiting.
    And(Box<Expr>, Box<Expr>),
    Or(Box<Expr>, Box<Expr>),
    /// Convert the operand to this expression's `ty`.
    Cast(Box<Expr>),
    Call(FuncIdx, Vec<Arg>),
    /// A struct with every field given, in declaration order.
    StructLit(StructIdx, Vec<Expr>),
    /// An enum value: the variant and its fields in declaration order.
    VariantLit(EnumIdx, u32, Vec<Expr>),
    /// The tag (variant number, as a u8) of the enum stored at a place.
    Tag(Place),
    /// A field of a struct value that isn't a place (e.g. a call's result).
    FieldOf(Box<Expr>, u32),
    /// Runs the statements, then yields the value. Used to evaluate
    /// labelled arguments in source order.
    Seq(Vec<Stmt>, Box<Expr>),
    If {
        cond: Box<Expr>,
        then_body: Vec<Stmt>,
        then_val: Option<Box<Expr>>,
        else_body: Vec<Stmt>,
        else_val: Option<Box<Expr>>,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub enum Arg {
    Value(Expr),
    /// Pass a pointer to this place (for a `var` parameter).
    Ref(Place),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnOp {
    Neg,
    Not,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Rem,
    Eq,
    Ne,
    Lt,
    Gt,
    Le,
    Ge,
}

impl BinOp {
    pub fn is_comparison(self) -> bool {
        matches!(self, BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Gt | BinOp::Le | BinOp::Ge)
    }
}
