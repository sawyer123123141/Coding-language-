use crate::span::Span;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ExprId(pub u32);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct BindId(pub u32);

#[derive(Clone, Debug)]
pub struct Program {
    pub items: Vec<Item>,
    /// Total `ExprId`s handed out; side tables size by this.
    pub expr_count: u32,
    pub bind_count: u32,
}

#[derive(Clone, Debug)]
pub enum Item {
    Fn(FnDecl),
    Const(ConstDecl),
    Struct(StructDecl),
    Enum(EnumDecl),
    Use(UseDecl),
}

/// `use math`, `use { noise2, lerp from math }`, or several clauses in braces.
#[derive(Clone, Debug)]
pub struct UseDecl {
    pub clauses: Vec<UseClause>,
    pub span: Span,
}

/// One module: with `names`, those come into scope unqualified; without,
/// the module's `pub` items are reached as `module.name`.
#[derive(Clone, Debug)]
pub struct UseClause {
    pub module: String,
    pub module_span: Span,
    pub names: Vec<(String, Span)>,
}

#[derive(Clone, Debug)]
pub struct EnumDecl {
    pub is_pub: bool,
    pub name: String,
    pub name_span: Span,
    pub variants: Vec<VariantDecl>,
    pub methods: Vec<FnDecl>,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub struct VariantDecl {
    pub name: String,
    pub name_span: Span,
    /// Named fields; empty for a unit variant. Fields never have defaults.
    pub fields: Vec<FieldDecl>,
}

#[derive(Clone, Debug)]
pub struct StructDecl {
    pub is_pub: bool,
    pub name: String,
    pub name_span: Span,
    pub fields: Vec<FieldDecl>,
    pub methods: Vec<FnDecl>,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub struct FieldDecl {
    pub name: String,
    pub name_span: Span,
    pub ty: TypeExpr,
    pub default: Option<Expr>,
}

#[derive(Clone, Debug)]
pub struct FnDecl {
    pub name: String,
    pub name_span: Span,
    pub is_pure: bool,
    pub is_pub: bool,
    pub params: Vec<Param>,
    pub ret: Option<TypeExpr>,
    pub body: Block,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub struct Param {
    pub bind: Binding,
    pub is_var: bool,
    /// `self` / `var self`: the type is the enclosing struct, filled in by the parser.
    pub is_self: bool,
    pub ty: TypeExpr,
}

#[derive(Clone, Debug)]
pub struct ConstDecl {
    pub is_pub: bool,
    pub bind: Binding,
    pub ty: Option<TypeExpr>,
    pub value: Expr,
    pub span: Span,
}

/// A name being introduced: `let` target, parameter, loop variable.
#[derive(Clone, Debug)]
pub struct Binding {
    pub id: BindId,
    pub name: String,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub struct TypeExpr {
    pub kind: TypeKind,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub enum TypeKind {
    Named { name: String, args: Vec<TypeExpr> },
    Array(Box<TypeExpr>),
    Optional(Box<TypeExpr>),
}

#[derive(Clone, Debug)]
pub struct Block {
    pub stmts: Vec<Stmt>,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub enum Stmt {
    /// `let x = v`, or `let x = opt else { ... }`, which unwraps an optional
    /// and runs the `else` block (which must leave) when it's `none`.
    Let { is_var: bool, bind: Binding, ty: Option<TypeExpr>, value: Expr, else_block: Option<Block>, span: Span },
    Assign { target: Expr, op: AssignOp, value: Expr, span: Span },
    While { cond: Expr, body: Block, span: Span },
    ForRange { var: Binding, start: Expr, end: Expr, body: Block, span: Span },
    /// `for v in xs` / `for i, v in xs`
    ForIn { index: Option<Binding>, var: Binding, iter: Expr, body: Block, span: Span },
    /// `arena { ... }`: everything allocated inside is freed at `}`.
    Arena { body: Block, span: Span },
    /// `in x.mem { ... }`: allocations inside go to the arena at `x.mem`.
    In { arena: Expr, body: Block, span: Span },
    Break(Span),
    Continue(Span),
    Return { value: Option<Expr>, span: Span },
    Expr(Expr),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AssignOp {
    Set,
    Add,
    Sub,
    Mul,
    Div,
    Rem,
}

#[derive(Clone, Debug)]
pub struct Expr {
    pub id: ExprId,
    pub kind: ExprKind,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub enum ExprKind {
    Int(u64),
    Float(f64),
    Bool(bool),
    /// `none`: an optional with no value.
    None,
    Str(String),
    /// `"hp = {hp}"`: literal text and expressions, in order.
    Interp(Vec<InterpPart>),
    Ident(String),
    Unary { op: UnaryOp, expr: Box<Expr> },
    Binary { op: BinaryOp, lhs: Box<Expr>, rhs: Box<Expr> },
    Call { name: String, name_span: Span, args: Vec<Arg> },
    If { cond: Box<Expr>, then_block: Block, else_block: Option<Block> },
    /// `base.name`
    Field { base: Box<Expr>, name: String, name_span: Span },
    /// `receiver.name(args)`
    MethodCall { receiver: Box<Expr>, name: String, name_span: Span, args: Vec<Arg> },
    Match { scrutinee: Box<Expr>, arms: Vec<MatchArm> },
    /// `base[index]`
    Index { base: Box<Expr>, index: Box<Expr> },
    /// `[a, b, c]`
    ArrayLit(Vec<Expr>),
    /// `array[int](n)`, `list[vec3]()`: a built-in generic type, called.
    TypeCall { name: String, name_span: Span, type_args: Vec<TypeExpr>, args: Vec<Arg> },
}

#[derive(Clone, Debug)]
pub enum InterpPart {
    Lit(String),
    Expr(Expr),
}

#[derive(Clone, Debug)]
pub struct MatchArm {
    pub pattern: Pattern,
    pub body: Block,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub enum Pattern {
    /// `Variant` or `Variant binding`.
    Variant { name: String, name_span: Span, binding: Option<Binding> },
    Else(Span),
}

#[derive(Clone, Debug)]
pub struct Arg {
    pub label: Option<String>,
    pub is_var: bool,
    pub value: Expr,
    pub span: Span,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnaryOp {
    Neg,
    Not,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BinaryOp {
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
    And,
    Or,
}

impl BinaryOp {
    pub fn symbol(self) -> &'static str {
        match self {
            BinaryOp::Add => "+",
            BinaryOp::Sub => "-",
            BinaryOp::Mul => "*",
            BinaryOp::Div => "/",
            BinaryOp::Rem => "%",
            BinaryOp::Eq => "==",
            BinaryOp::Ne => "!=",
            BinaryOp::Lt => "<",
            BinaryOp::Gt => ">",
            BinaryOp::Le => "<=",
            BinaryOp::Ge => ">=",
            BinaryOp::And => "and",
            BinaryOp::Or => "or",
        }
    }

    pub fn is_comparison(self) -> bool {
        matches!(self, BinaryOp::Eq | BinaryOp::Ne | BinaryOp::Lt | BinaryOp::Gt | BinaryOp::Le | BinaryOp::Ge)
    }
}

impl AssignOp {
    pub fn symbol(self) -> &'static str {
        match self {
            AssignOp::Set => "=",
            AssignOp::Add => "+=",
            AssignOp::Sub => "-=",
            AssignOp::Mul => "*=",
            AssignOp::Div => "/=",
            AssignOp::Rem => "%=",
        }
    }

    /// The arithmetic a compound assignment performs, if any.
    pub fn binary(self) -> Option<BinaryOp> {
        match self {
            AssignOp::Set => None,
            AssignOp::Add => Some(BinaryOp::Add),
            AssignOp::Sub => Some(BinaryOp::Sub),
            AssignOp::Mul => Some(BinaryOp::Mul),
            AssignOp::Div => Some(BinaryOp::Div),
            AssignOp::Rem => Some(BinaryOp::Rem),
        }
    }
}

impl Stmt {
    pub fn span(&self) -> Span {
        match self {
            Stmt::Let { span, .. }
            | Stmt::Assign { span, .. }
            | Stmt::While { span, .. }
            | Stmt::ForRange { span, .. }
            | Stmt::ForIn { span, .. }
            | Stmt::Arena { span, .. }
            | Stmt::In { span, .. }
            | Stmt::Return { span, .. } => *span,
            Stmt::Break(span) | Stmt::Continue(span) => *span,
            Stmt::Expr(e) => e.span,
        }
    }

}
