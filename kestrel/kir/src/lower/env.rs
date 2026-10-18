//! Closure environments. A closure literal with captures is a function value whose environment
//! holds a copy of each captured value: a block in the current arena (`EnvKind::Heap`) with a
//! 16-byte header (the address of the environment's copy function, then the block's size) and
//! the values after it, at the offsets of its `EnvLayout`. The values are deep copies (`owned`),
//! so nothing in the block shares memory with the variables it was made from.
//!
//! Inside the closure's code each capture is a reference local bound to its place in the
//! environment (`Stmt::BindEnv`), like a parameter passed by pointer: reading it into a new home
//! or returning it copies it, so `let g = () { xs }` hands out a new list at each call. The body's
//! reads of captures are found through `Tables::capture_reads`.
//!
//! Copying a function value (`fixup` of a `KTy::Fn`, `Stmt::CopyFnEnv`) calls the copy function
//! in its environment's header, which copies the block (`ExprKind::EnvDup`) and then what each
//! capture holds, into the current arena. So a copy shares nothing with the original either.

use kestrel_check::{Capture, Tables};
use kestrel_syntax::ast::ExprId;

use super::helpers::{expr, Helpers};
use super::{kty, layout, FnLowerer};
use crate::ir::*;

/// The code being lowered is closure `id`'s, and these locals are bound to its captures.
pub(crate) struct Captured {
    pub(crate) id: ExprId,
    pub(crate) locals: Vec<LocalIdx>,
}

/// Where the captures of closure literal `id` go in its environment.
pub(crate) fn env_layout(t: &Tables, id: ExprId) -> EnvLayout {
    let mut offsets = Vec::new();
    let mut end = ENV_HEADER;
    for c in &t.closures[&id].captures {
        let (size, align) = layout::size_align(t, kty(c.ty));
        end = end.next_multiple_of(align);
        offsets.push(end);
        end += size;
    }
    EnvLayout { offsets, size: end.next_multiple_of(8) }
}

/// A local of type `ty` that points at a capture, named for the captured variable.
fn capture_local(t: &Tables, c: &Capture) -> Local {
    Local { name: format!("{}_capture", t.local(c.root).name), ty: kty(c.ty), kind: LocalKind::RefParam }
}

/// `BindEnv` of each local in `locals` to its capture's offset in the environment at `env`.
fn bind_captures(env: LocalIdx, locals: &[LocalIdx], layout: &EnvLayout) -> Vec<Stmt> {
    let at = || expr(ExprKind::Load(Place::local(env)), KTy::I64);
    locals.iter().zip(&layout.offsets).map(|(&local, &offset)| Stmt::BindEnv { local, env: at(), offset }).collect()
}

impl Helpers {
    /// The copy function of closure literal `id`'s environment, made at the first use:
    /// `(env: i64) -> i64`, which copies the block and then each capture's own memory into the
    /// current arena, and returns the copy. One per literal.
    pub(crate) fn env_copy_fn(&mut self, id: ExprId, t: &Tables) -> FuncIdx {
        if let Some(&f) = self.env_copies.get(&id) {
            return f;
        }
        let idx = self.reserve();
        self.env_copies.insert(id, idx);
        let (env, copy) = (LocalIdx(0), LocalIdx(1));
        let mut locals = vec![
            Local { name: "env".into(), ty: KTy::I64, kind: LocalKind::Value },
            Local { name: "copy".into(), ty: KTy::I64, kind: LocalKind::Value },
        ];
        let captures = &t.closures[&id].captures;
        let refs: Vec<LocalIdx> = captures
            .iter()
            .map(|c| {
                locals.push(capture_local(t, c));
                LocalIdx(locals.len() as u32 - 1)
            })
            .collect();
        let layout = env_layout(t, id);
        let dup = expr(ExprKind::EnvDup(Box::new(expr(ExprKind::Load(Place::local(env)), KTy::I64))), KTy::I64);
        let mut body = vec![Stmt::Let(copy, dup)];
        body.extend(bind_captures(copy, &refs, &layout));
        for (c, &r) in captures.iter().zip(&refs) {
            body.extend(self.fixup(Place::local(r), kty(c.ty), t));
        }
        body.push(Stmt::Return(Some(expr(ExprKind::Load(Place::local(copy)), KTy::I64))));
        self.funcs[(idx.0 - self.base) as usize] = Func {
            name: format!("__kestrel_env_copy_{}", idx.0),
            params: vec![env],
            ret: Some(KTy::I64),
            locals,
            body,
            is_pure: true,
            io: false,
            mutates_params: false,
            exported: false,
            native: None,
        };
        idx
    }
}

impl FnLowerer<'_> {
    /// Gives the closure being lowered (literal `id`) a reference local for each capture, and
    /// returns the statements that bind them, which start its body.
    pub(crate) fn bind_closure_captures(&mut self, id: ExprId) -> Vec<Stmt> {
        let t = self.t;
        let mut locals = Vec::new();
        for c in &t.closures[&id].captures {
            locals.push(LocalIdx(self.locals.len() as u32));
            self.locals.push(capture_local(t, c));
        }
        // The environment word is the code's first parameter.
        let binds = bind_captures(LocalIdx(0), &locals, &env_layout(t, id));
        self.captured = Some(Captured { id, locals });
        binds
    }

    /// The place in this closure's environment that the capture read `e` names (see
    /// `Tables::capture_reads`), when `e` is one.
    pub(crate) fn capture_place(&self, e: ExprId) -> Option<Place> {
        let (id, k, rest) = self.t.capture_reads.get(&e)?;
        let cap = self.captured.as_ref().expect("a capture is read inside its closure's code");
        assert_eq!(cap.id, *id, "a capture read belongs to the closure being lowered");
        let mut p = Place::local(cap.locals[*k as usize]);
        p.path.extend(rest.iter().map(|&f| Proj::Field(f)));
        Some(p)
    }

    /// The place capture `c` of a closure made here reads: a capture of the closure being
    /// lowered (in its environment), or a place of the code here.
    fn capture_source(&self, c: &Capture) -> Place {
        let fields = |p: &mut Place, path: &[u32]| p.path.extend(path.iter().map(|&f| Proj::Field(f)));
        if let Some(cap) = &self.captured {
            let outer = &self.t.closures[&cap.id].captures;
            if let Some(k) = outer.iter().position(|o| o.root == c.root && c.path.starts_with(&o.path)) {
                let mut p = Place::local(cap.locals[k]);
                fields(&mut p, &c.path[outer[k].path.len()..]);
                return p;
            }
        }
        // A `match` binding names the fields of the variant stored at the matched place.
        if let Some((matched, variant)) = self.bindings.get(&c.root) {
            let mut p = matched.clone();
            p.path.push(Proj::Variant(*variant, c.path[0]));
            fields(&mut p, &c.path[1..]);
            return p;
        }
        // A loop item or a `let ... else` view stands for the place it reads.
        let mut p = match self.elems.get(&c.root) {
            Some(p) => p.clone(),
            None => Place::local(self.local_map[&c.root]),
        };
        fields(&mut p, &c.path);
        p
    }

    /// The environment of closure literal `id`, which captures something: a new block in the
    /// current arena holding a deep copy of each captured value, read now.
    pub(crate) fn heap_env(&mut self, id: ExprId, line: u32) -> EnvKind {
        let t = self.t;
        let copy = self.helpers.env_copy_fn(id, t);
        let values = t.closures[&id]
            .captures
            .iter()
            .map(|c| {
                let v = Expr { kind: ExprKind::Load(self.capture_source(c)), ty: kty(c.ty), line };
                self.owned(v, false)
            })
            .collect();
        EnvKind::Heap(Env { copy, layout: env_layout(t, id), values })
    }
}
