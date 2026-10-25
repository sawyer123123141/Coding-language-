//! Heap constants. A constant whose type holds an array, list, map or interface value is built
//! once, before `main`, into a global, and every use reads that one value in place. (A constant of
//! numbers, strings, vectors or plain structs allocates nothing, so each use still writes it out.)
//!
//! A function that uses a heap constant gets a reference local for it, pointed at the global
//! by a `BindGlobal` at the start of the body. A use is then a read of that local, which every
//! rule about places already handles: indexing, `.len`, loops and read-only arguments read it
//! in place, and storing it copies it (a reference local is never moved from).
//!
//! `__kestrel_constants` builds the constants that are used, and the ones they use, into an
//! arena that is never freed. It does nothing the second time it runs.

use std::collections::{HashMap, HashSet};

use kestrel_check::{ConstId, Res, Tables};
use kestrel_syntax::ast::{self, Program};
use kestrel_syntax::LineIndex;

use super::helpers::Helpers;
use super::{kty, FnLowerer};
use crate::ir::*;

/// The "built" flag and the constants' arena come first among the globals.
const BUILT: GlobalIdx = GlobalIdx(0);
const ARENA: GlobalIdx = GlobalIdx(1);
const FIRST_CONST: u32 = 2;

/// The globals of the heap constants the functions use.
#[derive(Default)]
pub(crate) struct ConstGlobals {
    /// The global of each constant used so far.
    index: HashMap<ConstId, GlobalIdx>,
    /// The constants in the order they were first used; the one at `i` has global `2 + i`.
    used: Vec<ConstId>,
    types: Vec<KTy>,
}

impl ConstGlobals {
    /// The global of constant `c`, which has type `ty`; allocated the first time.
    pub(crate) fn global(&mut self, c: ConstId, ty: KTy) -> GlobalIdx {
        if let Some(&g) = self.index.get(&c) {
            return g;
        }
        let g = GlobalIdx(FIRST_CONST + self.types.len() as u32);
        self.index.insert(c, g);
        self.used.push(c);
        self.types.push(ty);
        g
    }

    /// `Module::globals`: nothing when no heap constant is used.
    pub(crate) fn into_globals(self) -> Vec<KTy> {
        if self.types.is_empty() {
            return Vec::new();
        }
        [KTy::Bool, KTy::Arena].into_iter().chain(self.types).collect()
    }
}

/// Whether constant `c` is built once and read in place (`Tables::const_read_in_place`): its type
/// holds an array, list, map or interface value somewhere inside, and isn't owning.
pub(crate) fn hoisted(t: &Tables, c: ConstId) -> bool {
    t.const_read_in_place(t.consts[c.0 as usize].ty)
}

/// The declaration of constant `c`.
pub(super) fn decl<'p>(program: &'p Program, t: &Tables, c: ConstId) -> &'p ast::ConstDecl {
    let ast::Item::Const(decl) = &program.items[t.consts[c.0 as usize].item] else { unreachable!("ConstInfo points at a const") };
    decl
}

impl FnLowerer<'_> {
    /// A fresh reference local, to be pointed at something.
    fn ref_local(&mut self, name: &str, ty: KTy) -> LocalIdx {
        let idx = LocalIdx(self.locals.len() as u32);
        self.locals.push(Local { name: name.to_string(), ty, kind: LocalKind::RefParam });
        idx
    }

    /// The reference local for heap constant `c` in this function, made at the first use.
    pub(crate) fn const_local(&mut self, c: ConstId) -> LocalIdx {
        if let Some(&local) = self.const_locals.get(&c) {
            return local;
        }
        let t = self.t;
        let info = &t.consts[c.0 as usize];
        let ty = kty(info.ty);
        self.helpers.consts.global(c, ty);
        let local = self.ref_local(&info.name, ty);
        self.const_locals.insert(c, local);
        local
    }

    /// The statements that point the constants' locals at their globals, by local index so the
    /// output doesn't depend on hashing. They go first in the function.
    pub(crate) fn const_binds(&self) -> Vec<Stmt> {
        let mut binds: Vec<(LocalIdx, GlobalIdx)> =
            self.const_locals.iter().map(|(c, &local)| (local, self.helpers.consts.index[c])).collect();
        binds.sort_by_key(|&(local, _)| local);
        binds.into_iter().map(|(local, global)| Stmt::BindGlobal(local, global)).collect()
    }
}

/// Builds `__kestrel_constants` once every function is lowered, if any function uses a heap
/// constant, and gives its index. It is added after the functions and helpers lowered so far.
pub(super) fn lower_init(program: &Program, t: &Tables, lines: &LineIndex, helpers: &mut Helpers) -> Option<FuncIdx> {
    if helpers.consts.used.is_empty() {
        return None;
    }
    let order = build_order(program, t, &helpers.consts.used);
    let mut cx = FnLowerer::new(program, t, lines, None, helpers);
    let built = cx.ref_local("built", KTy::Bool);
    let arena = cx.ref_local("arena", KTy::Arena);
    let locals: Vec<LocalIdx> = order.iter().map(|&c| cx.const_local(c)).collect();

    // The whole build is inside one `if`, with no `return`, so the function is still correct if
    // the inliner splices it into `main`: an early return there would end `main`.
    let flag = |ty, kind| Expr { kind, ty, line: 0 };
    let loaded = flag(KTy::Bool, ExprKind::Load(Place::local(built)));
    let mut build = vec![
        Stmt::Assign(Place::local(built), flag(KTy::Bool, ExprKind::Bool(true))),
        // Never freed: the constants live as long as the program. (Not counted by `KESTREL_ARENA_COUNT`.)
        Stmt::Assign(Place::local(arena), flag(KTy::Arena, ExprKind::StaticArena)),
        Stmt::PushArena(Some(Place::local(arena)), 0),
    ];
    for (&c, &local) in order.iter().zip(&locals) {
        let value = cx.value(&decl(program, t, c).value);
        build.push(Stmt::Assign(Place::local(local), value));
    }
    build.push(Stmt::PopArena { free: false });
    let mut body = vec![Stmt::BindGlobal(built, BUILT), Stmt::BindGlobal(arena, ARENA)];
    body.extend(cx.const_binds());
    let cond = flag(KTy::Bool, ExprKind::Unary(UnOp::Not, Box::new(loaded)));
    body.push(Stmt::If { cond, then_body: build, else_body: Vec::new() });
    // This checks membership only, not order: the order itself comes from `build_order`. A
    // constant outside it would have no local bound at the start of the init.
    assert!(
        cx.const_locals.keys().all(|c| order.contains(c)),
        "a constant used while building constants is missing from the build order"
    );

    let func = Func {
        name: "__kestrel_constants".into(),
        params: Vec::new(),
        ret: None,
        locals: cx.locals,
        body,
        is_pure: false,
        io: false,
        mutates_params: false,
        exported: false,
        lines: (0, 0),
        native: None,
    };
    // Lowering the values added helpers (copies), so the init comes after them.
    let idx = FuncIdx(helpers.base + helpers.funcs.len() as u32);
    helpers.repoint(&func.body, idx);
    helpers.funcs.push(func);
    Some(idx)
}

/// The constants to build, each after the ones it uses: the constants `roots` (the ones the
/// functions use) and every constant they use, in turn, that holds an array, list or map.
/// Where nothing decides, the checker's order (`const_order`) does.
fn build_order(program: &Program, t: &Tables, roots: &[ConstId]) -> Vec<ConstId> {
    let rank: HashMap<ConstId, usize> = t.const_order.iter().enumerate().map(|(i, &c)| (c, i)).collect();
    let by_rank = |list: &mut Vec<ConstId>| list.sort_by_key(|c| rank[c]);
    let mut roots = roots.to_vec();
    by_rank(&mut roots);
    let (mut seen, mut order) = (HashSet::new(), Vec::new());
    for c in roots {
        visit(program, t, c, &by_rank, &mut seen, &mut order);
    }
    order.retain(|&c| hoisted(t, c));
    order
}

/// Adds `c` to `order` after the constants it uses.
fn visit(
    program: &Program,
    t: &Tables,
    c: ConstId,
    by_rank: &impl Fn(&mut Vec<ConstId>),
    seen: &mut HashSet<ConstId>,
    order: &mut Vec<ConstId>,
) {
    if !seen.insert(c) {
        return;
    }
    let mut uses = Vec::new();
    uses_in(program, t, &decl(program, t, c).value, &mut uses);
    by_rank(&mut uses);
    for used in uses {
        visit(program, t, used, by_rank, seen, order);
    }
    order.push(c);
}

/// The constants the constant expression `e` names, including the ones that the defaults of
/// the struct fields it leaves out name (`Palette()` with `ids: [int] = DEFAULT_IDS`).
fn uses_in(program: &Program, t: &Tables, e: &ast::Expr, out: &mut Vec<ConstId>) {
    let all = |args: &[ast::Arg], out: &mut Vec<ConstId>| args.iter().for_each(|a| uses_in(program, t, &a.value, out));
    match &e.kind {
        ast::ExprKind::Ident(_) => {
            if let Some(&Res::Const(c)) = t.res.get(&e.id) {
                out.push(c);
            }
        }
        ast::ExprKind::Call { args, .. } => {
            all(args, out);
            if let Some(&Res::Construct(sid)) = t.res.get(&e.id) {
                // Built-in structs (the vectors) have no declaration, and no defaults.
                if let Some(ast::Item::Struct(decl)) = program.items.get(t.strukt(sid).item) {
                    let slots = &t.ctor_args[&e.id];
                    for (field, _) in decl.fields.iter().zip(slots).filter(|(_, slot)| slot.is_none()) {
                        if let Some(default) = &field.default {
                            uses_in(program, t, default, out);
                        }
                    }
                }
            }
        }
        // A method call in a constant expression is an enum value (`Kit.Bag([3, 4])`): its payload
        // may name a heap constant that must be built first.
        ast::ExprKind::TypeCall { args, .. } | ast::ExprKind::MethodCall { args, .. } => all(args, out),
        ast::ExprKind::Unary { expr, .. } => uses_in(program, t, expr, out),
        // A swizzle of a constant (`ORIGIN.xy`).
        ast::ExprKind::Field { base, .. } => uses_in(program, t, base, out),
        ast::ExprKind::Binary { lhs, rhs, .. } => {
            uses_in(program, t, lhs, out);
            uses_in(program, t, rhs, out);
        }
        ast::ExprKind::ArrayLit(items) => items.iter().for_each(|i| uses_in(program, t, i, out)),
        ast::ExprKind::MapLit(entries) => entries.iter().for_each(|(k, v)| {
            uses_in(program, t, k, out);
            uses_in(program, t, v, out);
        }),
        ast::ExprKind::TypeApply { .. } => unreachable!("generics are lowered from Task 6b"),
        ast::ExprKind::Brackets { .. } => unreachable!("settled after parsing"),
        // Not in a constant expression (see `is_const_expr`).
        _ => {}
    }
}
