//! Constants: top-level `let NAME = ...`, whose initializers may only use
//! literals, operators, pure built-ins, struct constructions and other
//! constants (in any file, in any order, as long as they don't loop).

use std::collections::HashMap;

use kestrel_syntax::ast::{Arg, ConstDecl, Expr, ExprKind, Item, Program};
use kestrel_syntax::{Diagnostic, Span};

use super::BodyChecker;
use crate::builtins::Builtin;
use crate::collect::resolve_type;
use crate::tables::{ConstId, Tables, BUILTIN_STRUCTS};
use crate::ty::Ty;

pub(crate) fn check_consts(program: &Program, t: &mut Tables, diags: &mut Vec<Diagnostic>) {
    let consts: Vec<&ConstDecl> = program
        .items
        .iter()
        .filter_map(|it| match it {
            Item::Const(c) => Some(c),
            Item::Fn(_) | Item::Struct(_) | Item::Enum(_) | Item::Use(_) => None,
        })
        .collect();
    // A constant is checked after the constants it uses, whichever file they are in
    // (`palette.SKY = draw.BLUE` works whatever the order of the `use` lines). Where
    // nothing decides, a module's constants (`draw.RED`) come before the entry
    // file's, and each group keeps its source order.
    let in_module = |c: &&ConstDecl| c.bind.name.contains('.');
    let mut roots: Vec<usize> = (0..consts.len()).filter(|&i| in_module(&consts[i])).collect();
    roots.extend((0..consts.len()).filter(|&i| !in_module(&consts[i])));
    let ids = consts.iter().enumerate().map(|(i, c)| (c.bind.name.as_str(), i)).collect();
    let mut order = Order { marks: vec![Mark::Todo; consts.len()], stack: Vec::new(), consts, ids };
    for root in roots {
        order.visit(root, t, diags);
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Mark {
    Todo,
    /// On the stack: its constants are being checked first.
    Active,
    Done,
}

/// A depth-first walk over which constant uses which.
struct Order<'a> {
    consts: Vec<&'a ConstDecl>,
    ids: HashMap<&'a str, usize>,
    marks: Vec<Mark>,
    /// The constants being visited, outermost first.
    stack: Vec<usize>,
}

impl Order<'_> {
    fn visit(&mut self, i: usize, t: &mut Tables, diags: &mut Vec<Diagnostic>) {
        if self.marks[i] != Mark::Todo {
            return;
        }
        self.marks[i] = Mark::Active;
        self.stack.push(i);
        let mut uses = Vec::new();
        names_used(&self.consts[i].value, &mut uses);
        for (name, span) in uses {
            let Some(&j) = self.ids.get(name.as_str()) else { continue };
            match self.marks[j] {
                Mark::Todo => self.visit(j, t, diags),
                Mark::Active => self.report_cycle(j, span, t, diags),
                Mark::Done => {}
            }
        }
        self.stack.pop();
        self.marks[i] = Mark::Done;
        check_const(self.consts[i], i, t, diags);
        t.const_order.push(ConstId(i as u32));
    }

    /// `back` is on the stack and the constant at the top uses it: a loop. The
    /// constants in it get the error type, so their uses don't add more errors.
    fn report_cycle(&mut self, back: usize, span: Span, t: &mut Tables, diags: &mut Vec<Diagnostic>) {
        let from = self.stack.iter().position(|&k| k == back).expect("an active constant is on the stack");
        let cycle = &self.stack[from..];
        let name = |k: usize| self.consts[k].bind.name.as_str();
        let msg = if cycle.len() == 1 {
            format!("constant `{}` uses itself; give it a value that doesn't", name(back))
        } else {
            let path: Vec<&str> = cycle.iter().chain([&back]).map(|&k| name(k)).collect();
            format!("constants can't use each other in a loop: {}; give one of them a value that doesn't use the others", path.join(" -> "))
        };
        diags.push(Diagnostic::error(span, msg));
        for &k in cycle {
            t.consts[k].ty = Ty::Error;
            t.const_by_name.insert(self.consts[k].bind.name.clone(), ConstId(k as u32));
        }
    }
}

/// The names `e` mentions, each once with where it first appears: the
/// constants it uses, when `e` is a constant expression (see `is_const_expr`).
fn names_used(e: &Expr, out: &mut Vec<(String, Span)>) {
    match &e.kind {
        ExprKind::Ident(name) => {
            if !out.iter().any(|(n, _)| n == name) {
                out.push((name.clone(), e.span));
            }
        }
        ExprKind::Call { args, .. } | ExprKind::TypeCall { args, .. } => args.iter().for_each(|a| names_used(&a.value, out)),
        ExprKind::Unary { expr, .. } => names_used(expr, out),
        ExprKind::Binary { lhs, rhs, .. } => {
            names_used(lhs, out);
            names_used(rhs, out);
        }
        ExprKind::ArrayLit(items) => items.iter().for_each(|i| names_used(i, out)),
        ExprKind::MapLit(entries) => entries.iter().for_each(|(k, v)| {
            names_used(k, out);
            names_used(v, out);
        }),
        _ => {}
    }
}

fn check_const(c: &ConstDecl, i: usize, t: &mut Tables, diags: &mut Vec<Diagnostic>) {
    let mut bc = BodyChecker::new(t, diags, None, Ty::Unit, false);
    let want = c.ty.as_ref().map(|ty| resolve_type(ty, bc.t, bc.diags));
    let ty = if is_const_expr(&c.value, bc.t, true) {
        bc.check_value(&c.value, want)
    } else {
        bc.error(c.value.span, "constants can only use literals, operators and other constants");
        Ty::Error
    };
    t.consts[i].ty = ty;
    t.const_by_name.insert(c.bind.name.clone(), ConstId(i as u32));
}

/// Whether `e` is a constant expression. A constant may build structs
/// (`user_structs`); a field default may build only vectors, because a default
/// is built again at every construction: one that builds a struct could
/// build itself forever, and one that holds memory could outlive its arena.
pub(crate) fn is_const_expr(e: &Expr, t: &Tables, user_structs: bool) -> bool {
    let all = |args: &[Arg]| args.iter().all(|a| is_const_expr(&a.value, t, user_structs));
    match &e.kind {
        ExprKind::Int(_) | ExprKind::Float(_) | ExprKind::Bool(_) | ExprKind::None | ExprKind::Str(_) | ExprKind::Ident(_) => true,
        ExprKind::Call { name, args, .. } if name == "Arena" => args.is_empty(),
        // Pure built-ins (`sqrt(2.0)`) are fine; the random ones change state.
        ExprKind::Call { name, args, .. } if Builtin::global(name).is_some_and(Builtin::is_pure) => all(args),
        // A struct construction (`Color(1, 0, 0)`, `vec2(0, 1)`) is a value like any other.
        ExprKind::Call { name, args, .. } if t.struct_by_name.get(name).is_some_and(|s| user_structs || s.0 < BUILTIN_STRUCTS) => all(args),
        ExprKind::Unary { expr, .. } => is_const_expr(expr, t, user_structs),
        ExprKind::Binary { lhs, rhs, .. } => is_const_expr(lhs, t, user_structs) && is_const_expr(rhs, t, user_structs),
        // Each use builds a fresh array, like any other value.
        ExprKind::ArrayLit(items) => items.iter().all(|i| is_const_expr(i, t, user_structs)),
        ExprKind::MapLit(entries) => entries.iter().all(|(k, v)| is_const_expr(k, t, user_structs) && is_const_expr(v, t, user_structs)),
        ExprKind::TypeCall { args, .. } => all(args),
        ExprKind::Interp(_)
        | ExprKind::Index { .. }
        | ExprKind::Call { .. }
        | ExprKind::If { .. }
        | ExprKind::Field { .. }
        | ExprKind::MethodCall { .. }
        | ExprKind::Match { .. } => false,
    }
}

/// Whether `e` builds a struct of the program's own (`Color(1, 1, 1)`), anywhere inside it.
pub(crate) fn builds_user_struct(e: &Expr, t: &Tables) -> bool {
    let any = |args: &[Arg]| args.iter().any(|a| builds_user_struct(&a.value, t));
    match &e.kind {
        ExprKind::Call { name, args, .. } => t.struct_by_name.get(name).is_some_and(|s| s.0 >= BUILTIN_STRUCTS) || any(args),
        ExprKind::TypeCall { args, .. } => any(args),
        ExprKind::Unary { expr, .. } => builds_user_struct(expr, t),
        ExprKind::Binary { lhs, rhs, .. } => builds_user_struct(lhs, t) || builds_user_struct(rhs, t),
        ExprKind::ArrayLit(items) => items.iter().any(|i| builds_user_struct(i, t)),
        ExprKind::MapLit(entries) => entries.iter().any(|(k, v)| builds_user_struct(k, t) || builds_user_struct(v, t)),
        _ => false,
    }
}

/// Whether the constant expression `e` names a constant that is, or holds, a
/// struct of the program's own (not a vector).
pub(crate) fn uses_struct_constant(e: &Expr, t: &Tables) -> bool {
    let any = |args: &[Arg]| args.iter().any(|a| uses_struct_constant(&a.value, t));
    match &e.kind {
        ExprKind::Ident(name) => t.const_by_name.get(name).is_some_and(|c| holds_user_struct(t, t.consts[c.0 as usize].ty)),
        ExprKind::Call { args, .. } | ExprKind::TypeCall { args, .. } => any(args),
        ExprKind::Unary { expr, .. } => uses_struct_constant(expr, t),
        ExprKind::Binary { lhs, rhs, .. } => uses_struct_constant(lhs, t) || uses_struct_constant(rhs, t),
        ExprKind::ArrayLit(items) => items.iter().any(|i| uses_struct_constant(i, t)),
        ExprKind::MapLit(entries) => entries.iter().any(|(k, v)| uses_struct_constant(k, t) || uses_struct_constant(v, t)),
        _ => false,
    }
}

fn holds_user_struct(t: &Tables, ty: Ty) -> bool {
    match ty {
        Ty::Struct(s) => s.0 >= BUILTIN_STRUCTS,
        Ty::Array(e) | Ty::List(e) | Ty::Optional(e) | Ty::Map(_, e) => holds_user_struct(t, t.elem(e)),
        Ty::Enum(_) => true,
        _ => false,
    }
}
