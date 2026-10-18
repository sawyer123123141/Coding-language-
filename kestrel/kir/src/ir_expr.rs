//! KIR expressions, call arguments and operators.

use crate::ir::*;

#[derive(Clone, Debug, PartialEq)]
pub struct Expr {
    pub kind: ExprKind,
    pub ty: KTy,
    /// Source line and file, packed (see `pack_line`), for runtime error messages.
    pub line: u32,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ExprKind {
    Int(i64),
    Float(f64),
    Bool(bool),
    /// A string literal; the bytes live in read-only data.
    StrLit(String),
    /// An interpolated string `"hp {hp}"`: its parts printed as `print` would
    /// print them, built into a string in the current arena. The parts are
    /// only `PrintArg::Value` and `PrintArg::Str`, since lowering flattens a
    /// nested interpolation into its parent.
    Format(Vec<PrintArg>),
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
    /// The length (i64) of the array or list at a place; of a string, its bytes.
    Len(Place),
    /// An operation on a string value; see `StrOp` for the arguments.
    Str(StrOp, Vec<Expr>),
    /// `Arena()`: a new, empty arena.
    NewArena,
    /// A new, empty arena that lives until the program ends and that `KESTREL_ARENA_COUNT` leaves
    /// out: the one the heap constants are built in.
    StaticArena,
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
    /// A float operation (see `MathOp`); integer `abs`/`min`/`max` lower to `If`.
    Math(MathOp, Vec<Expr>),
    /// A float from 0 up to (not including) 1, of this expression's type.
    RandFloat,
    /// An integer between the two operands, both included, of this expression's type.
    RandInt(Box<Expr>, Box<Expr>),
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
