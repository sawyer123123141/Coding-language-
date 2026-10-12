//! Constants: top-level `let NAME = ...`, whose initializers may only use
//! literals, operators, pure built-ins, struct and enum values and other
//! constants (in any file, in any order, as long as they don't loop).

use std::collections::HashMap;

use kestrel_syntax::ast::{Arg, ConstDecl, Expr, ExprKind, Item, Program};
use kestrel_syntax::{Diagnostic, Span};

use super::BodyChecker;
use crate::builtins::Builtin;
use crate::collect::resolve_type;
use crate::tables::{ConstId, Tables};
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
        ExprKind::Call { args, .. } | ExprKind::TypeCall { args, .. } | ExprKind::MethodCall { args, .. } => {
            args.iter().for_each(|a| names_used(&a.value, out))
        }
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
    let ty = if is_const_expr(&c.value, bc.t) {
        bc.check_value(&c.value, want)
    } else {
        bc.error(c.value.span, "constants can only use literals, operators, other constants, struct and enum values, and pure built-ins like `sqrt`");
        Ty::Error
    };
    t.consts[i].ty = ty;
    t.const_by_name.insert(c.bind.name.clone(), ConstId(i as u32));
}

/// Whether `e` is a constant expression: literals, operators, constants, pure built-ins, struct
/// and enum values, and array and map literals of those. Constants and field defaults are built
/// again where they're used, so both take these (field defaults also can't build a value that owns
/// an arena or a GPU object: see `check_field_defaults`; loops are found by `check_default_cycles`).
pub(crate) fn is_const_expr(e: &Expr, t: &Tables) -> bool {
    let all = |args: &[Arg]| args.iter().all(|a| is_const_expr(&a.value, t));
    // `State.Idle` and `Shape.Circle(1)`: an enum's values.
    let names_enum = |base: &Expr| matches!(&base.kind, ExprKind::Ident(n) if t.enum_by_name.contains_key(n));
    match &e.kind {
        ExprKind::Int(_) | ExprKind::Float(_) | ExprKind::Bool(_) | ExprKind::None | ExprKind::Str(_) | ExprKind::Ident(_) => true,
        ExprKind::Call { name, args, .. } if name == "Arena" => args.is_empty(),
        // Pure built-ins (`sqrt(2.0)`) are fine; the random ones change state.
        ExprKind::Call { name, args, .. } if Builtin::global(name).is_some_and(Builtin::is_pure) => all(args),
        // A struct construction (`Color(1, 0, 0)`, `vec2(0, 1)`) is a value like any other.
        ExprKind::Call { name, args, .. } if t.struct_by_name.contains_key(name) => all(args),
        ExprKind::Field { base, .. } => names_enum(base),
        ExprKind::MethodCall { receiver, args, .. } => names_enum(receiver) && all(args),
        ExprKind::Unary { expr, .. } => is_const_expr(expr, t),
        ExprKind::Binary { lhs, rhs, .. } => is_const_expr(lhs, t) && is_const_expr(rhs, t),
        // Each use builds a fresh array, like any other value.
        ExprKind::ArrayLit(items) => items.iter().all(|i| is_const_expr(i, t)),
        ExprKind::MapLit(entries) => entries.iter().all(|(k, v)| is_const_expr(k, t) && is_const_expr(v, t)),
        ExprKind::TypeCall { args, .. } => all(args),
        ExprKind::Interp(_) | ExprKind::Index { .. } | ExprKind::Call { .. } | ExprKind::If { .. } | ExprKind::Match { .. } => false,
    }
}
