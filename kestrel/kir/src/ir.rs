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
    /// A pointer to a runtime arena (null until created).
    Arena,
    /// `T?`: a u8 tag (0 = `none`) at offset 0, then the value.
    Optional(ElemIdx),
    /// `map[K, V]`: a 64-byte header (see `MapLayout`) over entries and an
    /// index table in the map's arena.
    Map(ElemIdx, ElemIdx),
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
                | KTy::Optional(_)
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
    /// `None` for a library (see `kestrel_check::check_with`).
    pub main: Option<FuncIdx>,
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
    /// Makes an arena current: a fresh one (`arena { }`), or the one at a
    /// place (`in x { }`, creating it there if it's still null).
    PushArena(Option<Place>),
    /// Undoes the latest `PushArena`, freeing the arena for `arena { }`.
    PopArena { free: bool },
    /// Frees the arena at a place (dropping it) and nulls the place.
    FreeArena(Place),
    /// Gives the array, list or map at a place its own copy of its
    /// elements, in the current arena (a list's capacity becomes its length).
    Dup(Place),
    /// `m[key] = value`: adds the key or replaces its value. The key, then
    /// the value, are evaluated first.
    MapSet(Place, Expr, Expr),
    /// Ends a loop over the map: its lock count goes back to `saved`.
    MapUnlock(Place, Expr),
    /// `par for`: runs `func` (a `par_bodies` entry) over `0..count` on
    /// many threads. Its parameters are `args` (pointers to the variables
    /// the body uses), then the start and end of a range of indexes.
    ParFor { func: FuncIdx, args: Vec<Arg>, count: Expr },
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
    /// `map[K, V]()`: empty, growing in the current arena.
    MapNew,
    /// An operation on the map at a place; see `MapOp` for the arguments.
    Map(MapOp, Place, Vec<Expr>),
    /// The length (i64) of the array or list at a place.
    Len(Place),
    /// `Arena()`: a new, empty arena.
    NewArena,
    /// An optional holding this value.
    Some(Box<Expr>),
    /// `none`
    NoneLit,
    /// The value at a place, which is then zeroed (an owning value moving
    /// out, so dropping the place later does nothing).
    Move(Place),
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

/// What `ExprKind::Map` does with the map at its place.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MapOp {
    /// `[key]`: an optional holding the value (bytes copied).
    Get,
    /// `[key]`: a bool.
    Has,
    /// `[key]`: removes the key; a bool, true when it was there.
    Remove,
    /// `[]`: the number of entries used, removed ones included (i64).
    Used,
    /// `[index]`: whether entry `index` is below `Used` and still there.
    Live,
    /// `[]`: starts a loop over the map; the old lock count (i64), for `MapUnlock`.
    Lock,
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
    /// Every bit flipped (integers).
    BitNot,
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
    BitAnd,
    BitOr,
    BitXor,
    /// Both operands have the result's type (lowering converts the amount).
    Shl,
    Shr,
}

impl BinOp {
    pub fn is_comparison(self) -> bool {
        matches!(self, BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Gt | BinOp::Le | BinOp::Ge)
    }
}
