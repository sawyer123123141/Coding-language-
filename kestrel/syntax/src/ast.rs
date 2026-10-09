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
    Let { is_var: bool, bind: Binding, ty: Option<TypeExpr>, value: Expr, span: Span },
    Assign { target: Expr, op: AssignOp, value: Expr, span: Span },
    While { cond: Expr, body: Block, span: Span },
    ForRange { var: Binding, start: Expr, end: Expr, body: Block, span: Span },
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
    Str(String),
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
            | Stmt::Return { span, .. } => *span,
            Stmt::Break(span) | Stmt::Continue(span) => *span,
            Stmt::Expr(e) => e.span,
        }
    }

    pub fn to_sexpr(&self) -> String {
        match self {
            Stmt::Let { is_var, bind, value, .. } => {
                format!("({} {} {})", if *is_var { "var" } else { "let" }, bind.name, value.to_sexpr())
            }
            Stmt::Assign { target, op, value, .. } => {
                format!("({} {} {})", op.symbol(), target.to_sexpr(), value.to_sexpr())
            }
            Stmt::While { cond, body, .. } => format!("(while {} {})", cond.to_sexpr(), body.to_sexpr()),
            Stmt::ForRange { var, start, end, body, .. } => format!(
                "(for {} {} {} {})",
                var.name,
                start.to_sexpr(),
                end.to_sexpr(),
                body.to_sexpr()
            ),
            Stmt::Break(_) => "break".into(),
            Stmt::Continue(_) => "continue".into(),
            Stmt::Return { value: None, .. } => "return".into(),
            Stmt::Return { value: Some(v), .. } => format!("(return {})", v.to_sexpr()),
            Stmt::Expr(e) => e.to_sexpr(),
        }
    }
}

impl Block {
    pub fn to_sexpr(&self) -> String {
        let inner: Vec<String> = self.stmts.iter().map(Stmt::to_sexpr).collect();
        format!("{{{}}}", inner.join("; "))
    }
}

impl Expr {
    /// Compact prefix form for tests: `1 + 2 * 3` is `(+ 1 (* 2 3))`.
    pub fn to_sexpr(&self) -> String {
        match &self.kind {
            ExprKind::Int(n) => n.to_string(),
            ExprKind::Float(f) => f.to_string(),
            ExprKind::Bool(b) => b.to_string(),
            ExprKind::Str(s) => format!("{s:?}"),
            ExprKind::Ident(name) => name.clone(),
            ExprKind::Unary { op, expr } => {
                let sym = match op {
                    UnaryOp::Neg => "-",
                    UnaryOp::Not => "not",
                };
                format!("({sym} {})", expr.to_sexpr())
            }
            ExprKind::Binary { op, lhs, rhs } => format!("({} {} {})", op.symbol(), lhs.to_sexpr(), rhs.to_sexpr()),
            ExprKind::Call { name, args, .. } => format!("(call {name}{})", args_sexpr(args)),
            ExprKind::Field { base, name, .. } => format!("(. {} {name})", base.to_sexpr()),
            ExprKind::MethodCall { receiver, name, args, .. } => {
                format!("(.call {} {name}{})", receiver.to_sexpr(), args_sexpr(args))
            }
            ExprKind::Match { scrutinee, arms } => {
                let mut s = format!("(match {}", scrutinee.to_sexpr());
                for arm in arms {
                    let pat = match &arm.pattern {
                        Pattern::Variant { name, binding: Some(b), .. } => format!("{name} {}", b.name),
                        Pattern::Variant { name, binding: None, .. } => name.clone(),
                        Pattern::Else(_) => "else".to_string(),
                    };
                    s.push_str(&format!(" [{pat} {}]", arm.body.to_sexpr()));
                }
                s.push(')');
                s
            }
            ExprKind::If { cond, then_block, else_block } => match else_block {
                Some(e) => format!("(if {} {} else {})", cond.to_sexpr(), then_block.to_sexpr(), e.to_sexpr()),
                None => format!("(if {} {})", cond.to_sexpr(), then_block.to_sexpr()),
            },
        }
    }
}

fn args_sexpr(args: &[Arg]) -> String {
    let mut s = String::new();
    for a in args {
        s.push(' ');
        if let Some(l) = &a.label {
            s.push_str(&format!("{l}: "));
        }
        if a.is_var {
            s.push_str("var ");
        }
        s.push_str(&a.value.to_sexpr());
    }
    s
}
