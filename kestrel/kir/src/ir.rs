//! Kestrel's typed, structured middle layer. Loops stay loops; no
//! Cranelift (or any backend) types appear here.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KTy {
    Bool,
    U8,
    I32,
    I64,
    F32,
    F64,
    /// A struct, laid out by `Module::structs`. Lives in memory, never in a register.
    Struct(StructIdx),
}

impl KTy {
    pub fn is_aggregate(self) -> bool {
        matches!(self, KTy::Struct(_))
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

#[derive(Clone, Debug, PartialEq)]
pub struct Module {
    pub funcs: Vec<Func>,
    pub structs: Vec<StructLayout>,
    pub main: FuncIdx,
}

impl Module {
    pub fn layout(&self, s: StructIdx) -> &StructLayout {
        &self.structs[s.0 as usize]
    }

    /// Size and alignment in bytes.
    pub fn size_align(&self, ty: KTy) -> (u32, u32) {
        match ty {
            KTy::Bool | KTy::U8 => (1, 1),
            KTy::I32 | KTy::F32 => (4, 4),
            KTy::I64 | KTy::F64 => (8, 8),
            KTy::Struct(s) => {
                let l = self.layout(s);
                (l.size, l.align)
            }
        }
    }
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

/// A memory location: a local, or a field path inside one.
#[derive(Clone, Debug, PartialEq)]
pub struct Place {
    pub root: LocalIdx,
    /// Field indices from the root inward.
    pub fields: Vec<u32>,
}

impl Place {
    pub fn local(root: LocalIdx) -> Self {
        Place { root, fields: Vec::new() }
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
    /// struct parameter (read-only ones are passed by pointer too).
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
}

#[derive(Clone, Debug, PartialEq)]
pub enum PrintArg {
    Value(Expr),
    Str(String),
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
