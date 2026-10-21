//! One walk over a function, in the order the code runs it (a node after its operands), that
//! finds everything the report lists besides the checks left: what allocates, what can fail, and
//! the calls. Each candidate was checked against its arm in codegen and the runtime function it
//! calls (`runtime/src/list.rs`, `arena.rs`, `map.rs`, `string*.rs`, `par.rs`).

mod reasons;
mod stmts;

use reasons::{elem_text, limit, place_ty};
use super::show::{expr_text, place_text};
use super::summary::Reason;
use crate::ir::*;

/// What a function does that the report lists.
#[derive(Clone, Debug, PartialEq)]
pub struct Event {
    /// The packed line it is at (the latest line seen, for code with none of its own).
    pub line: u32,
    /// The packed lines of the loops around it, outermost first (generated loops have no line and
    /// are left out).
    pub loops: Vec<u32>,
    pub what: What,
}

#[derive(Clone, Debug, PartialEq)]
pub enum What {
    Alloc(Reason),
    Fail(Reason),
    /// A call to a function of the program (a native's call is its own `Alloc` and `Fail`).
    Call(FuncIdx),
    /// A `par for` that runs this body.
    Body(FuncIdx),
    /// A closure literal is made here: its code may run wherever the function value goes, so it
    /// counts as called (an edge for summaries only; its own items are listed with the function
    /// it is written in).
    Closure(FuncIdx),
}

/// How deep the walk goes: past this the code is left unlisted (the analysis gave up on it long
/// before).
const MAX_DEPTH: u32 = 4096;

/// The events of function `f`, in the order the code runs them.
pub fn events(m: &Module, f: &Func) -> Vec<Event> {
    let mut w = Walk { m, f, out: Vec::new(), loops: Vec::new(), cur: f.lines.0, depth: 0, unused: unused_makes(f) };
    w.stmts(&f.body);
    w.out
}

struct Walk<'a> {
    m: &'a Module,
    f: &'a Func,
    out: Vec<Event>,
    loops: Vec<u32>,
    /// The packed line of the latest expression: the line of code that has none.
    cur: u32,
    depth: u32,
    /// Closure literals nothing can call (see `unused_makes`), by address.
    unused: std::collections::HashSet<usize>,
}

/// The closure literals in `f` that nothing can call: `let g = <literal>` where `g` is only ever
/// read for its environment word (`FnEnv`) while no call to the code remains. That is what is left of a literal passed to a built-in
/// that was inlined: the loop calls the code inlined, and the value stays behind, unused. By the
/// address of the literal's expression.
pub(super) fn unused_makes(f: &Func) -> std::collections::HashSet<usize> {
    use std::collections::HashMap;
    let mut lets: Vec<(LocalIdx, usize, FuncIdx)> = Vec::new();
    let mut called: std::collections::HashSet<FuncIdx> = std::collections::HashSet::new();
    let (mut places, mut envs): (HashMap<LocalIdx, u32>, HashMap<LocalIdx, u32>) = (HashMap::new(), HashMap::new());
    crate::visit::walk(&f.body, &mut |n| match n {
        crate::visit::Node::Stmt(Stmt::Let(l, e @ Expr { kind: ExprKind::MakeFn { code, .. }, .. })) => lets.push((*l, e as *const Expr as usize, *code)),
        crate::visit::Node::Expr(Expr { kind: ExprKind::Call(g, _), .. }) | crate::visit::Node::Stmt(Stmt::Call(g, _, _)) => {
            called.insert(*g);
        }
        crate::visit::Node::Place(p, _) => *places.entry(p.root).or_default() += 1,
        crate::visit::Node::Expr(Expr { kind: ExprKind::FnEnv(p), .. }) if p.path.is_empty() => *envs.entry(p.root).or_default() += 1,
        _ => {}
    });
    // (A loop that stays a call to a closure too big to inline still calls the code.)
    lets.into_iter().filter(|(l, _, code)| places.get(l) == envs.get(l) && !called.contains(code)).map(|(_, at, _)| at).collect()
}


impl Walk<'_> {
    fn here(&self, line: u32) -> u32 {
        if line != 0 {
            line
        } else {
            self.cur
        }
    }

    fn push(&mut self, line: u32, what: What) {
        let line = self.here(line);
        self.out.push(Event { line, loops: self.loops.clone(), what });
    }

    fn reason(&self, line: u32, direct: Option<String>, via: String, tail: &str) -> Reason {
        Reason { direct, via, tail: tail.to_string(), line: self.here(line) }
    }

    fn alloc(&mut self, line: u32, direct: Option<String>, via: String, tail: &str) {
        let r = self.reason(line, direct, via, tail);
        self.push(line, What::Alloc(r));
    }

    fn fail(&mut self, line: u32, direct: Option<String>, via: String, tail: &str) {
        let r = self.reason(line, direct, via, tail);
        self.push(line, What::Fail(r));
    }

    /// The built-in a generated function, or a local an inlined one made, belongs to.
    fn builtin_of(&self, root: LocalIdx) -> Option<&'static str> {
        let name = &self.f.locals[root.0 as usize].name;
        let owner = name.rsplit_once('.').map_or(self.f.name.as_str(), |(o, _)| o);
        super::report::builtin_name(owner)
    }

    /// `p` for a sentence: in backticks, or "the list" for code the compiler made or a value it
    /// holds in a temporary.
    fn place(&self, p: &Place) -> String {
        let mut text = place_text(self.m, self.f, p, p.path.len());
        if text.starts_with("(...)") && p.path.is_empty() {
            // A value the compiler held in a temporary: what was put in it, if that is known.
            if let Some(def) = self.definition(p.root) {
                text = expr_text(self.m, self.f, def);
            }
        }
        if self.f.lines == (0, 0) || text.starts_with("(...)") {
            let noun = match place_ty(self.m, self.f, p) {
                KTy::List(_) => "list",
                KTy::Array(_) => "array",
                KTy::Map(..) => "map",
                KTy::Str => "string",
                _ => "value",
            };
            // A field of something the compiler holds: the field's name says which.
            return match (p.path.last(), text.rsplit_once('.')) {
                (Some(Proj::Field(_) | Proj::Variant(..)), Some((_, field))) => format!("the {noun} `{field}`"),
                _ => format!("the {noun}"),
            };
        }
        format!("`{text}`")
    }

    /// What the temporary `l` is set to, when it is set once.
    fn definition(&self, l: LocalIdx) -> Option<&Expr> {
        let mut found: Vec<&Expr> = Vec::new();
        crate::visit::walk(&self.f.body, &mut |n| {
            if let crate::visit::Node::Stmt(Stmt::Let(m, e)) = n {
                if *m == l {
                    found.push(e);
                }
            }
        });
        (found.len() == 1).then(|| found[0])
    }

    /// `x` for a sentence: its text, or what its temporary was set to.
    pub(super) fn expr_name(&self, x: &Expr) -> String {
        let text = expr_text(self.m, self.f, x);
        match &x.kind {
            ExprKind::Load(p) if text == "(...)" && p.path.is_empty() => self.definition(p.root).map_or(text, |d| expr_text(self.m, self.f, d)),
            _ => text,
        }
    }

    fn expr(&mut self, e: &Expr) {
        if self.depth >= MAX_DEPTH {
            return;
        }
        self.depth += 1;
        if e.line != 0 {
            self.cur = e.line;
        }
        self.expr_inner(e);
        self.depth -= 1;
    }

    fn expr_inner(&mut self, e: &Expr) {
        let line = e.line;
        match &e.kind {
            ExprKind::Int(_)
            | ExprKind::Float(_)
            | ExprKind::Bool(_)
            | ExprKind::StrLit(_)
            | ExprKind::ListNew
            | ExprKind::MapNew
            | ExprKind::NoneLit
            | ExprKind::FnRef(_)
            | ExprKind::RandFloat => {}
            ExprKind::MakeFn { code, env: EnvKind::None } => self.closure(e, *code),
            ExprKind::StaticArena => self.alloc(line, Some("makes the arena the constants live in".to_string()), "makes the arena the constants live in".to_string(), ""),
            ExprKind::NewArena => self.alloc(line, Some("`Arena()` makes a new arena".to_string()), "makes a new arena".to_string(), ""),
            ExprKind::Format(parts) => {
                self.print(parts);
                self.alloc(line, Some("string interpolation makes a new string".to_string()), "builds a string with interpolation".to_string(), "");
            }
            ExprKind::ArrayNew(n) => {
                self.expr(n);
                let KTy::Array(el) = e.ty else { return };
                let name = format!("array[{}](...)", elem_text(self.m, self.m.elem(el)));
                self.alloc(line, Some(format!("`{name}` makes a new array")), "makes a new array".to_string(), "");
                let tail = format!(" and fails if its length is negative or more than {}", limit());
                self.fail(line, Some(format!("`{name}` fails if its length is negative or more than {}", limit())), "makes a new array".to_string(), &tail);
            }
            ExprKind::ArrayLit(items) => {
                for x in items {
                    self.expr(x);
                }
                if !items.is_empty() {
                    let what = if matches!(e.ty, KTy::List(_)) { "list" } else { "array" };
                    self.alloc(line, Some(format!("`[...]` makes a new {what}")), format!("makes a new {what}"), "");
                }
            }
            ExprKind::Map(_, p, xs) => {
                for x in xs {
                    self.expr(x);
                }
                self.place_walk(p);
            }
            ExprKind::Len(p) | ExprKind::Tag(p) | ExprKind::FnEnv(p) | ExprKind::Load(p) | ExprKind::Move(p) => self.place_walk(p),
            ExprKind::Str(op, xs) => {
                for x in xs {
                    self.expr(x);
                }
                self.str_op(*op, xs, line);
            }
            ExprKind::Some(x) | ExprKind::Unary(_, x) | ExprKind::Cast(x) | ExprKind::Proven(x) | ExprKind::FieldOf(x, _) => self.expr(x),
            ExprKind::EnvDup(x) => {
                self.expr(x);
                self.alloc(line, Some("copies a closure's environment".to_string()), "copies a closure's environment".to_string(), "");
            }
            ExprKind::BlockDup { src, .. } => {
                self.expr(src);
                self.alloc(line, Some("copies an interface value's data".to_string()), "copies an interface value's data".to_string(), "");
            }
            ExprKind::Binary(_, a, b) | ExprKind::And(a, b) | ExprKind::Or(a, b) | ExprKind::RandInt(a, b) => {
                self.expr(a);
                self.expr(b);
            }
            ExprKind::Math(_, xs) | ExprKind::StructLit(_, xs) | ExprKind::VariantLit(_, _, xs) => {
                for x in xs {
                    self.expr(x);
                }
            }
            ExprKind::MakeFn { code, env: EnvKind::Heap(env) } => {
                for v in &env.values {
                    self.expr(v);
                }
                self.closure(e, *code);
                self.alloc(line, Some("a closure that holds captured values makes an environment for them".to_string()), "makes a closure that holds captured values".to_string(), "");
            }
            ExprKind::MakeFn { code, env: EnvKind::Stack(env) } => {
                for v in &env.values {
                    self.expr(v);
                }
                self.closure(e, *code);
            }
            ExprKind::Call(g, args) => {
                self.args(args);
                self.call(*g, line);
            }
            ExprKind::CallIndirect { callee, args, .. } => {
                self.place_walk(callee);
                self.args(args);
                self.indirect(callee, line);
            }
            ExprKind::CallVirtual { recv, args, .. } => {
                self.place_walk(recv);
                self.args(args);
                self.virtual_call(recv, line);
            }
            ExprKind::MakeIface { data: IfaceData::Borrowed(p), .. } => self.place_walk(p),
            ExprKind::MakeIface { data: IfaceData::Boxed(x), .. } => {
                self.expr(x);
                self.alloc(line, Some("an interface value makes a box for its data".to_string()), "makes an interface value".to_string(), "");
            }
            ExprKind::Seq(stmts, v) => {
                self.stmts(stmts);
                self.expr(v);
            }
            ExprKind::If { cond, then_body, then_val, else_body, else_val } => {
                self.expr(cond);
                self.stmts(then_body);
                if let Some(v) = then_val {
                    self.expr(v);
                }
                self.stmts(else_body);
                if let Some(v) = else_val {
                    self.expr(v);
                }
            }
        }
    }
}
