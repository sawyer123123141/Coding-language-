//! What a statement or expression may write. Every place KIR's walker marks as written
//! (`visit::places`) is a whole write, unless it is one of the known list operations below; so a
//! write added to KIR later is covered as long as the walker marks it.

use std::collections::HashMap;

use crate::ir::*;
use crate::visit::{walk, walk_expr, Node};

/// How a list's length changes.
#[derive(Clone, Debug, PartialEq)]
pub enum LenChange {
    /// By this many items (`add` and `insert` 1, `remove_at` -1).
    Add(i64),
    /// To this value (`clear` 0, `SetLen` its operand).
    Set(Expr),
    /// To something not known (`Compact`, the end of a `remove b` loop).
    Unknown,
}

#[derive(Clone, Debug, PartialEq)]
pub enum WriteKind {
    /// Anything in the place may change.
    Whole,
    /// The items change, the length doesn't (`sort`, `Dup`).
    Items,
    /// The length changes (and the items with it).
    Length(LenChange),
}

#[derive(Clone, Debug, PartialEq)]
pub enum Write {
    /// A local gets a new value: `let`, a `for` range's variable, or a reference local pointed at
    /// another place (`Bind`, `BindEnv`, `BindGlobal`).
    Local(LocalIdx),
    /// A place is written (an assignment, a move, a `var` argument, a lent interface value, ...).
    Place(Place, WriteKind),
}

impl Write {
    /// The local whose own value this write may change: a local's new value, or a place rooted at
    /// it (an `Addressed` integer local passed as `var`, for one).
    pub fn root(&self) -> LocalIdx {
        match self {
            Write::Local(l) => *l,
            Write::Place(p, _) => p.root,
        }
    }
}

/// Everything `s` may write, its own effect included.
pub fn of_stmt(s: &Stmt) -> Vec<Write> {
    let mut c = Collector::default();
    walk(std::slice::from_ref(s), &mut |n| c.node(n));
    c.out
}

/// Everything evaluating `e` may write.
pub fn of_expr(e: &Expr) -> Vec<Write> {
    let mut c = Collector::default();
    c.expr(e);
    c.out
}

/// What evaluating the operands of a simple statement may write: the writes of its expressions,
/// including the index expressions of its places and the values of its arguments, but not the
/// statement's own effect (the place it assigns, the list it changes, its `var` arguments).
pub fn of_operands(s: &Stmt) -> Vec<Write> {
    let mut c = Collector::default();
    for e in operands(s) {
        c.expr(e);
    }
    c.out
}

/// The expressions a simple statement evaluates before its effect, in no particular order: its
/// values, the values of its arguments, and the index expressions of its places. A compound
/// statement (`If`, `While`, `ForRange`) has none here: its parts are analysed by their own rules.
pub fn operands(s: &Stmt) -> Vec<&Expr> {
    let mut out = Vec::new();
    match s {
        Stmt::Let(_, e) | Stmt::Return(Some(e)) | Stmt::RandSeed(e) => out.push(e),
        Stmt::Assign(p, e) | Stmt::ListAdd(p, e) | Stmt::SetLen(p, e) => {
            out.push(e);
            index_exprs(p, &mut out);
        }
        Stmt::List(_, p, xs) => {
            out.extend(xs);
            index_exprs(p, &mut out);
        }
        Stmt::MapSet(p, k, v, _) => {
            out.extend([k, v]);
            index_exprs(p, &mut out);
        }
        Stmt::MapUnlock(p, saved) => {
            out.push(saved);
            index_exprs(p, &mut out);
        }
        Stmt::Call(_, args) => args_of(args, &mut out),
        Stmt::CallIndirect { callee: p, args, .. } | Stmt::CallVirtual { recv: p, args, .. } => {
            index_exprs(p, &mut out);
            args_of(args, &mut out);
        }
        Stmt::ParFor { args, count, .. } => {
            out.push(count);
            args_of(args, &mut out);
        }
        Stmt::Print(parts) | Stmt::Write(parts) => print_values(parts, &mut out),
        Stmt::BindEnv { env, .. } => out.push(env),
        Stmt::Bind(_, p)
        | Stmt::CopyFnEnv(p)
        | Stmt::CopyIface(p)
        | Stmt::Dup(p)
        | Stmt::FreeArena(p)
        | Stmt::DropGpu(p)
        | Stmt::PushArena(Some(p)) => index_exprs(p, &mut out),
        Stmt::If { .. }
        | Stmt::While { .. }
        | Stmt::ForRange { .. }
        | Stmt::Break
        | Stmt::Continue
        | Stmt::Return(None)
        | Stmt::BindGlobal(..)
        | Stmt::PushArena(None)
        | Stmt::PopArena { .. }
        | Stmt::SetLine(_)
        | Stmt::Trap
        | Stmt::Fail(_) => {}
    }
    out
}

fn index_exprs<'a>(p: &'a Place, out: &mut Vec<&'a Expr>) {
    out.extend(p.path.iter().filter_map(Proj::at));
}

/// The value arguments, and the index expressions of the `var` arguments' places.
fn args_of<'a>(args: &'a [Arg], out: &mut Vec<&'a Expr>) {
    for a in args {
        match a {
            Arg::Value(e) => out.push(e),
            Arg::Ref(p) => index_exprs(p, out),
        }
    }
}

fn print_values<'a>(parts: &'a [PrintArg], out: &mut Vec<&'a Expr>) {
    for p in parts {
        match p {
            PrintArg::Value(e) => out.push(e),
            PrintArg::Interp(inner) => print_values(inner, out),
            PrintArg::Str(_) => {}
        }
    }
}

#[derive(Default)]
struct Collector {
    out: Vec<Write>,
    /// The kind of the next write of a place, by the place's address: set by the list statement
    /// that owns the place, which the walker reports before the place itself.
    kinds: HashMap<usize, WriteKind>,
}

impl Collector {
    fn expr(&mut self, e: &Expr) {
        walk_expr(e, &mut |n| self.node(n));
    }

    fn node(&mut self, n: Node) {
        match n {
            Node::Stmt(s) => {
                match s {
                    Stmt::Let(l, _) | Stmt::ForRange { var: l, .. } | Stmt::Bind(l, _) | Stmt::BindGlobal(l, _) => {
                        self.out.push(Write::Local(*l))
                    }
                    Stmt::BindEnv { local, .. } => self.out.push(Write::Local(*local)),
                    _ => {}
                }
                if let Some((p, kind)) = list_change(s) {
                    self.kinds.insert(p as *const Place as usize, kind);
                }
            }
            Node::Place(p, true) => {
                let kind = self.kinds.remove(&(p as *const Place as usize)).unwrap_or(WriteKind::Whole);
                self.out.push(Write::Place(p.clone(), kind));
            }
            Node::Place(_, false) | Node::Expr(_) => {}
        }
    }
}

/// The list a statement changes and how, for the list operations whose change is known.
fn list_change(s: &Stmt) -> Option<(&Place, WriteKind)> {
    Some(match s {
        Stmt::ListAdd(p, _) => (p, WriteKind::Length(LenChange::Add(1))),
        Stmt::List(ListOp::Insert, p, _) => (p, WriteKind::Length(LenChange::Add(1))),
        Stmt::List(ListOp::RemoveAt, p, _) => (p, WriteKind::Length(LenChange::Add(-1))),
        Stmt::List(ListOp::Clear, p, _) => (p, WriteKind::Length(LenChange::Set(Expr { kind: ExprKind::Int(0), ty: KTy::I64, line: 0 }))),
        Stmt::SetLen(p, n) => (p, WriteKind::Length(LenChange::Set(n.clone()))),
        Stmt::List(ListOp::Compact, p, _) => (p, WriteKind::Length(LenChange::Unknown)),
        Stmt::List(ListOp::Sort, p, _) | Stmt::Dup(p) => (p, WriteKind::Items),
        _ => return None,
    })
}
