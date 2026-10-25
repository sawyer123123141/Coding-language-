//! Field defaults and constants are built again at each use, so ones that build each other would
//! never finish (`kids: list[Node] = [Node()]`, or through a constant). This finds such loops
//! before KIR, whose constant ordering and construction would otherwise recurse forever.

use std::collections::{HashMap, HashSet};

use kestrel_syntax::ast::{Expr, ExprKind, Item, Program};
use kestrel_syntax::{Diagnostic, Span};

use crate::tables::{ConstId, Res, StructId, Tables};

/// Something built from an expression at each use: a field's default, or a constant.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum Node {
    Default(StructId, usize),
    Const(ConstId),
}

#[derive(Clone, Copy, PartialEq)]
enum Mark {
    /// On the stack: what it builds is being walked.
    Active,
    Done,
}

/// Reports each loop of defaults (and the constants they name) that build each other. Run it
/// after `check_field_defaults`, and only without earlier errors: it needs `ctor_args`.
pub(crate) fn check_default_cycles(program: &Program, t: &Tables, diags: &mut Vec<Diagnostic>) {
    let mut walk = Walk { program, t, marks: HashMap::new(), stack: Vec::new() };
    for (sid, s) in t.structs.iter().enumerate() {
        for fi in 0..s.fields.len() {
            walk.visit(Node::Default(StructId(sid as u32), fi), diags);
        }
    }
}

struct Walk<'a> {
    program: &'a Program,
    t: &'a Tables,
    marks: HashMap<Node, Mark>,
    /// The nodes being walked, outermost first.
    stack: Vec<Node>,
}

impl<'a> Walk<'a> {
    fn visit(&mut self, n: Node, diags: &mut Vec<Diagnostic>) {
        if self.marks.contains_key(&n) {
            return;
        }
        let Some(e) = self.expr(n) else { return };
        self.marks.insert(n, Mark::Active);
        self.stack.push(n);
        let mut next = Vec::new();
        self.builds(e, &mut next);
        // Building the same thing twice (`[Node(), Node()]`) is one loop, not two.
        let mut seen = HashSet::new();
        next.retain(|m| seen.insert(*m));
        for m in next {
            match self.marks.get(&m) {
                None => self.visit(m, diags),
                Some(Mark::Active) => self.report(m, diags),
                Some(Mark::Done) => {}
            }
        }
        self.stack.pop();
        self.marks.insert(n, Mark::Done);
    }

    /// The expression `n` is built from; `None` for a field with no default (or a built-in struct's).
    fn expr(&self, n: Node) -> Option<&'a Expr> {
        match n {
            Node::Default(sid, fi) => match self.program.items.get(self.t.strukt(sid).item)? {
                Item::Struct(decl) => decl.fields.get(fi)?.default.as_ref(),
                _ => None,
            },
            Node::Const(c) => match self.program.items.get(self.t.consts[c.0 as usize].item)? {
                Item::Const(decl) => Some(&decl.value),
                _ => None,
            },
        }
    }

    /// What building `e` also builds: the defaults of the fields a construction leaves out, and
    /// the constants it names.
    fn builds(&self, e: &'a Expr, out: &mut Vec<Node>) {
        match &e.kind {
            ExprKind::Ident(_) => {
                if let Some(&Res::Const(c)) = self.t.res.get(&e.id) {
                    out.push(Node::Const(c));
                }
            }
            ExprKind::Call { args, .. } => {
                if let (Some(&Res::Construct(sid)), Some(slots)) = (self.t.res.get(&e.id), self.t.ctor_args.get(&e.id)) {
                    out.extend(slots.iter().enumerate().filter(|(_, s)| s.is_none()).map(|(fi, _)| Node::Default(sid, fi)));
                }
                args.iter().for_each(|a| self.builds(&a.value, out));
            }
            ExprKind::MethodCall { args, .. } | ExprKind::TypeCall { args, .. } => args.iter().for_each(|a| self.builds(&a.value, out)),
            ExprKind::Unary { expr, .. } => self.builds(expr, out),
            // A component or swizzle of a constant vector builds that vector.
            ExprKind::Field { base, .. } => self.builds(base, out),
            ExprKind::Binary { lhs, rhs, .. } => {
                self.builds(lhs, out);
                self.builds(rhs, out);
            }
            ExprKind::ArrayLit(items) => items.iter().for_each(|i| self.builds(i, out)),
            ExprKind::MapLit(entries) => entries.iter().for_each(|(k, v)| {
                self.builds(k, out);
                self.builds(v, out);
            }),
            ExprKind::Brackets { .. } => unreachable!("settled after parsing"),
            _ => {}
        }
    }

    /// `back` is on the stack and the node on top builds it: a loop, reported where it closes.
    fn report(&self, back: Node, diags: &mut Vec<Diagnostic>) {
        let from = self.stack.iter().position(|&n| n == back).expect("an active node is on the stack");
        let path: Vec<String> = self.stack[from..].iter().chain([&back]).map(|&n| self.name(n)).collect();
        let span = self.expr(back).map_or(Span::default(), |e| e.span);
        let msg = format!(
            "field defaults build each other in a loop: {}; set the field when you build it, or start it empty (`[]`)",
            path.join(" -> ")
        );
        diags.push(Diagnostic::error(span, msg));
    }

    fn name(&self, n: Node) -> String {
        match n {
            Node::Default(sid, fi) => {
                let s = self.t.strukt(sid);
                format!("{}.{}", s.name, s.fields[fi].name)
            }
            Node::Const(c) => self.t.consts[c.0 as usize].name.clone(),
        }
    }
}
