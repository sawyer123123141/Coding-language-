use std::collections::HashMap;

use kestrel_syntax::ast::{BindId, ExprId};
use kestrel_syntax::Span;

use crate::ty::Ty;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct LocalId(pub u32);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct FnId(pub u32);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ConstId(pub u32);

/// What a name or call expression refers to.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Res {
    Local(LocalId),
    Const(ConstId),
    Fn(FnId),
    Print,
    Convert(Ty),
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
    pub is_pure: bool,
    pub params: Vec<ParamSig>,
    pub ret: Ty,
    pub span: Span,
    /// Every local owned by this function, parameters first in order.
    pub locals: Vec<LocalId>,
    pub io: bool,
    pub mutates_params: bool,
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
}
