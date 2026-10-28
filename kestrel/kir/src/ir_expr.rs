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
    /// An integer or `ivec4` `Binary(Add | Sub | Mul | Div | Rem | Shl | Shr, ..)`, `Unary(Neg, ..)`
    /// or narrowing integer `Cast` all of whose checks range analysis proved. For `/` and `%`
    /// that means the divisor is never 0 and never -1 with the dividend at its type's minimum,
    /// so codegen emits a plain divide. Only range analysis makes one, as KIR's last step.
    Proven(Box<Expr>),
    /// A float operation (see `MathOp`); integer `abs`/`min`/`max` lower to `If`.
    Math(MathOp, Vec<Expr>),
    /// A float from 0 up to (not including) 1, of this expression's type; `ranged` when it is
    /// the draw of a `randf_range` (contracts name what the user wrote).
    RandFloat { ranged: bool },
    /// An integer between the two operands, both included, of this expression's type.
    RandInt(Box<Expr>, Box<Expr>),
    Call(FuncIdx, Vec<Arg>),
    /// The function value of a named function: the address of its thunk (the function this
    /// index names; see `Helpers::thunk_for`) and no environment.
    FnRef(FuncIdx),
    /// The function value of a closure literal: the address of its code (the function this index
    /// names; see `Helpers::closure_code`) and the environment `env` builds. The code takes the
    /// environment word first, then the parameters.
    MakeFn { code: FuncIdx, env: EnvKind },
    /// The environment word (an `i64`) of the function value at a place.
    FnEnv(Place),
    /// A copy of the heap environment at the address the operand (an `i64`) gives, in the current
    /// arena: its size is in its header. The copy's address, an `i64`. Only an environment's copy
    /// function uses it (see `Helpers::env_copy_fn`), before it copies what the captures hold.
    EnvDup(Box<Expr>),
    /// A copy of the `size` bytes at the address `src` (an `i64`) gives, in a new block of the
    /// current arena aligned to `align`. The copy's address, an `i64`. Only an interface value's
    /// copy function uses it (see `Helpers::iface_copy_fn`), before it copies what the value holds.
    BlockDup { src: Box<Expr>, size: u32, align: u32 },
    /// A call through the function value at `callee`: its code gets the environment, then
    /// `args`, which follow `sig` as a direct call's follow the callee's parameters.
    CallIndirect { callee: Place, args: Vec<Arg>, sig: KSig },
    /// An interface value: the concrete value `data` names, and method table `table` (see
    /// `MethodTable`).
    MakeIface { table: TableIdx, data: IfaceData },
    /// A call of method `slot` of the interface value at `recv`, through its table: the method
    /// gets the result pointer (if any), the value's data pointer as `self`, then `args`, which
    /// follow `sig` (whose parameters leave out `self`). `keeps` is what the interface method's
    /// `@prove` promises (every implementation is checked to keep it), and `method` is the
    /// interface method's own function (its stub, `Shape.area`), for messages.
    CallVirtual { recv: Place, slot: u32, args: Vec<Arg>, sig: KSig, keeps: ContractSet, method: FuncIdx },
    /// A struct with every field given, in declaration order.
    StructLit(StructIdx, Vec<Expr>),
    /// An enum value: the variant and its fields in declaration order.
    VariantLit(EnumIdx, u32, Vec<Expr>),
    /// The tag (variant number, as a u8) of the enum stored at a place.
    Tag(Place),
    /// A field of a struct value that isn't a place (e.g. a call's result).
    FieldOf(Box<Expr>, u32),
    /// A vector (`ty`, `F32X4` or `I32X4`) of these lane values, one per lane, evaluated in order.
    VecLit(Vec<Expr>),
    /// A vector (`ty`) with the number (of its lane type) in every lane.
    Splat(Box<Expr>),
    /// A vector (`ty`) whose lane `i` is lane `.1[i]` of the vector `.0` (`v.wzyx`).
    Shuffle(Box<Expr>, [u8; 4]),
    /// Lane `.1` of a vector value that isn't a place (e.g. a call's result).
    LaneOf(Box<Expr>, u8),
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
    /// Both operands have the result's type (lowering converts the amount), except on a vector:
    /// its amount is one number of its lane type (`ivec4 << int`), shifting every lane. `>>` is
    /// arithmetic on signed types and `ivec4`.
    Shl,
    Shr,
    /// `*%`, `+%`, `-%` (integers): the result wraps around and nothing is ever checked, in
    /// every build.
    AddWrap,
    SubWrap,
    MulWrap,
}

impl BinOp {
    pub fn is_comparison(self) -> bool {
        matches!(self, BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Gt | BinOp::Le | BinOp::Ge)
    }

    /// `*% +% -%`.
    pub fn is_wrapping(self) -> bool {
        matches!(self, BinOp::AddWrap | BinOp::SubWrap | BinOp::MulWrap)
    }
}

/// What a closure literal's function value holds besides its code.
#[derive(Clone, Debug, PartialEq)]
pub enum EnvKind {
    /// Nothing: the closure captures nothing, and the environment word is 0.
    None,
    /// A new environment in the current arena, holding the captured values. The values are
    /// evaluated (in order) after the block is allocated and before the function value is
    /// written; each is a value of its own (a deep copy of the captured place).
    Heap(Env),
    /// An environment in a stack slot of the function making it, holding shallow copies of the
    /// captured values (a string's header, not its bytes), read with plain loads when the value
    /// is made. Only a closure literal that is directly a call's argument gets one
    /// (`Tables::stack_closures`), so its value appears only as an `Arg::Value` of a `Call`, a
    /// `CallIndirect` or a `CallVirtual` (see `check_stack_envs`). Its header holds the real copy function: a callee
    /// that stores its parameter copies it into a heap environment.
    Stack(Env),
}

/// Where an interface value's concrete value is.
#[derive(Clone, Debug, PartialEq)]
pub enum IfaceData {
    /// At this place, where it already is (lent to a call): the data pointer is the place's
    /// address, so a `var self` method called through the value changes the place. Like a stack
    /// environment, such a value appears only as an `Arg::Value` of a `Call`, a `CallIndirect` or
    /// a `CallVirtual` (see `check_stack_envs`), never in a temporary of its own, so it lives
    /// exactly as long as the call. `owned` turns it into a boxed one.
    Borrowed(Place),
    /// In a new block of the current arena, of the concrete value's size, holding this value
    /// (made independent with `owned` before it gets here).
    Boxed(Box<Expr>),
}

/// A closure's environment: a 16-byte header, then the captured values.
#[derive(Clone, Debug, PartialEq)]
pub struct Env {
    /// The function that copies the environment deep: `(env: i64) -> i64`, whose address the
    /// header holds at offset 0 (see `Helpers::env_copy_fn`).
    pub copy: FuncIdx,
    pub layout: EnvLayout,
    /// The captured values, one per offset.
    pub values: Vec<Expr>,
}

/// Where an environment's parts are: the header holds the copy function's address at offset 0
/// and `size` (an `i64`) at 8; the captured values follow at `offsets`, each at its natural
/// alignment. `size` counts the header and is a multiple of 8. Every block (made in an arena,
/// copied, or in a stack slot) is aligned to `ENV_ALIGN`, so a `vec4` capture at a multiple of 16
/// is 16-aligned in memory.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EnvLayout {
    pub offsets: Vec<u32>,
    pub size: u32,
}

/// The size of an environment's header: the copy function's address and the size.
pub const ENV_HEADER: u32 = 16;
/// The alignment of every environment block: the largest a capture may need (a `vec4`'s).
pub const ENV_ALIGN: u32 = 16;
