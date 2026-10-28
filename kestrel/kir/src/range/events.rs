//! One walk over a function, in the order the code runs it (a node after its operands), that
//! finds everything the report lists besides the checks left: what allocates, what can fail, and
//! the calls. Each candidate was checked against its arm in codegen and the runtime function it
//! calls (`runtime/src/list.rs`, `arena.rs`, `map.rs`, `string*.rs`, `par.rs`).

mod effects;
mod reasons;
mod stmts;

pub use effects::Effect;
use reasons::{elem_text, limit, place_ty};
use super::show::{expr_text, place_text};
use super::summary::{Part, Reason};
use crate::ir::*;

/// What a function does that the report lists.
#[derive(Clone, Debug, PartialEq)]
pub struct Event {
    /// The packed line it is at (the latest line seen, for code with none of its own).
    pub line: u32,
    /// The packed lines of the loops around it, outermost first (generated loops have no line and
    /// are left out).
    pub loops: Vec<u32>,
    /// The `Stmt::Prove` blocks around it, outermost first (see `ContractSite`).
    pub regions: Vec<Region>,
    pub what: What,
}

#[derive(Clone, Debug, PartialEq)]
pub enum What {
    Alloc(Reason),
    /// A way to fail; its reason's `part` says whether it is only running out of resources.
    Fail(Reason),
    /// A call whose body isn't known here: through a function value, an interface value or to a
    /// native. The reason is the report's words ("calls the function value `f`", line, "; its
    /// body isn't known here").
    Unknown(Unknown, Reason),
    /// Something that breaks `deterministic` (and nothing else): the reason is a chain's words
    /// ("calls `randf`", line, ", which uses the program's shared random generator"). A native or
    /// a call through a value has its `Unknown` event as well, for the other contracts.
    Effect(Effect, Reason),
    /// A call to a function of the program.
    Call(FuncIdx),
    /// A `par for` that runs this body.
    Body(FuncIdx),
    /// A closure literal is made here: its code may run wherever the function value goes, so it
    /// counts as called (an edge for summaries only; its own items are listed with the function
    /// it is written in).
    Closure(FuncIdx),
    /// The walk stopped here: the code nests more than `MAX_DEPTH` deep, and what is inside isn't
    /// listed (the analysis gave the function up long before).
    Cut,
}

/// A call whose body isn't known where it is made.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unknown {
    /// Through a function value: it may allocate, fail or have checks.
    Indirect,
    /// Through an interface value: it keeps what the interface method promises (the set), and the
    /// method is the interface's own (`Shape.area`).
    Virtual(ContractSet, FuncIdx),
    /// A native: Rust, with no checks of Kestrel's, but it may allocate and fail.
    Native,
}

impl Unknown {
    /// Whether the call may allocate (an item for the report, and `no_alloc`).
    pub fn allocates(self) -> bool {
        !matches!(self, Unknown::Virtual(keeps, _) if keeps.contains(ContractSet::NO_ALLOC))
    }

    /// Whether the call may stop the program (an item for the report, and `no_panic`).
    pub fn fails(self) -> bool {
        !matches!(self, Unknown::Virtual(keeps, _) if keeps.contains(ContractSet::NO_PANIC))
    }

    /// Whether the call may have a check left in it (`no_checks`).
    pub fn checks(self) -> bool {
        match self {
            Unknown::Indirect => true,
            Unknown::Virtual(keeps, _) => !keeps.contains(ContractSet::NO_CHECKS),
            Unknown::Native => false,
        }
    }
}

/// The words of a `par for`'s allocation: the arenas the runtime makes for its threads.
pub const PAR_ARENAS: &str = "a `par for` makes an arena for each thread";

/// How deep the walk goes: past this the code is left unlisted, with a `What::Cut` where it stops
/// (the analysis gave up on it long before).
const MAX_DEPTH: u32 = 4096;

/// The events of function `f`, in the order the code runs them.
pub fn events(m: &Module, f: &Func) -> Vec<Event> {
    let mut w = Walk { m, f, out: Vec::new(), loops: Vec::new(), regions: Vec::new(), copies: None, cur: f.lines.0, depth: 0, unused: unused_makes(f) };
    w.stmts(&f.body);
    w.out
}

struct Walk<'a> {
    m: &'a Module,
    f: &'a Func,
    out: Vec<Event>,
    loops: Vec<u32>,
    /// The `Prove` blocks around the current point, outermost first, and each block's number
    /// (made at the first one: see `prove_copies`).
    regions: Vec<Region>,
    copies: Option<std::collections::HashMap<usize, u32>>,
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
        self.out.push(Event { line, loops: self.loops.clone(), regions: self.regions.clone(), what });
    }

    /// The walk stops here (`MAX_DEPTH`): a `What::Cut`, once for a run of cut code in one region.
    fn cut(&mut self) {
        let again = self.out.last().is_some_and(|e| e.what == What::Cut && e.regions == self.regions);
        if !again {
            self.push(0, What::Cut);
        }
    }

    fn reason(&self, line: u32, direct: Option<String>, via: String, tail: &str) -> Reason {
        Reason { direct, via, tail: tail.to_string(), line: self.here(line), part: Part::Whole }
    }

    fn alloc(&mut self, line: u32, direct: Option<String>, via: String, tail: &str) {
        let r = self.reason(line, direct, via, tail);
        self.push(line, What::Alloc(r));
    }

    fn fail(&mut self, line: u32, direct: Option<String>, via: String, tail: &str) {
        self.fail_part(line, direct, via, tail, Part::Whole);
    }

    /// A way to fail that is only part of what one operation can do (see `Part`).
    fn fail_part(&mut self, line: u32, direct: Option<String>, via: String, tail: &str, part: Part) {
        let r = Reason { part, ..self.reason(line, direct, via, tail) };
        self.push(line, What::Fail(r));
    }

    /// A call whose body isn't known here: `via` is how it is named ("calls the function value
    /// `f`").
    fn unknown(&mut self, line: u32, kind: Unknown, via: String) {
        let r = self.reason(line, None, via, "; its body isn't known here");
        self.push(line, What::Unknown(kind, r));
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
            return self.cut();
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
            | ExprKind::FnRef(_) => {}
            // (`randf_range` is lowered to a `randf` too.)
            ExprKind::RandFloat => self.effect(line, Effect::SharedRandom, "calls `randf`".to_string()),
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
            ExprKind::Some(x)
            | ExprKind::Unary(_, x)
            | ExprKind::Cast(x)
            | ExprKind::Proven(x)
            | ExprKind::FieldOf(x, _)
            | ExprKind::Splat(x)
            | ExprKind::Shuffle(x, _)
            | ExprKind::LaneOf(x, _) => self.expr(x),
            ExprKind::EnvDup(x) => {
                self.expr(x);
                self.alloc(line, Some("copies a closure's environment".to_string()), "copies a closure's environment".to_string(), "");
            }
            ExprKind::BlockDup { src, .. } => {
                self.expr(src);
                self.alloc(line, Some("copies an interface value's data".to_string()), "copies an interface value's data".to_string(), "");
            }
            ExprKind::Binary(_, a, b) | ExprKind::And(a, b) | ExprKind::Or(a, b) => {
                self.expr(a);
                self.expr(b);
            }
            ExprKind::RandInt(a, b) => {
                self.expr(a);
                self.expr(b);
                self.effect(line, Effect::SharedRandom, "calls `randi_range`".to_string());
            }
            ExprKind::Math(_, xs) | ExprKind::StructLit(_, xs) | ExprKind::VariantLit(_, _, xs) | ExprKind::VecLit(xs) => {
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
            ExprKind::CallIndirect { callee, args, sig } => {
                self.place_walk(callee);
                self.args(args);
                self.indirect(callee, line, sig.pure);
            }
            ExprKind::CallVirtual { recv, args, keeps, method, sig, .. } => {
                self.place_walk(recv);
                self.args(args);
                self.virtual_call(recv, line, *keeps, *method, sig.pure);
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
