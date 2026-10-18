//! Captures: the places outside a closure literal that its body reads (see the closure spec's
//! "Captures"). A capture is a path: a local or parameter of the code around, followed through
//! fields as far as the body goes, and stopping at an index (`xs[i]` captures `xs` and `i`), a
//! built-in property (`xs.len` captures `xs`), a method call (`p.pos.length()` captures `p.pos`)
//! or a use of the whole value. A path inside another is covered by it (`p.pos` covers
//! `p.pos.x`). A call of a function field through a capture (`b.on(1)`) reads the field, `b.on`.
//!
//! **How they are found.** While a body is checked, each name of an outside local starts a read,
//! and a field access of a read's place extends it (the read moves up to the bigger expression).
//! When the literal ends, the reads that no other read covers become its captures, and each read
//! is recorded in `Tables::capture_reads` (which capture, and the fields left), for KIR. The
//! captures of a closure inside the body that come from outside the body too are reads of the
//! body as well: they are copied out of it when the inner closure is made.
//!
//! **The rules.** A capture can't change (`Immutable::Captured`) and can't be moved. It can't be
//! owning, so a closure is never owning. A closure that copies its captures (all of them, for
//! now: every closure with captures has a heap environment) can't capture a whole struct or enum
//! value that holds a function value: storing such a closure where the value is would copy the
//! old closure into the new one, without bound.

use std::collections::HashMap;

use kestrel_syntax::ast::{Expr, ExprId, ExprKind};
use kestrel_syntax::Span;

use super::lists::Step;
use super::BodyChecker;
use crate::closure::Capture;
use crate::tables::{LocalId, Res};
use crate::ty::Ty;

/// A read of a place outside the innermost closure literal being checked.
#[derive(Clone)]
struct CaptureRead {
    /// The whole read; `None` for a capture of a closure inside the body.
    expr: Option<ExprId>,
    root: LocalId,
    /// Field indexes, and the fields' names for messages.
    path: Vec<u32>,
    names: Vec<String>,
    ty: Ty,
    span: Span,
}

/// The reads of one closure literal's body, in the order they start.
#[derive(Default)]
pub(crate) struct Reads {
    list: Vec<CaptureRead>,
    /// The read each expression is, while a field access may still extend it.
    at: HashMap<ExprId, usize>,
}

impl CaptureRead {
    /// Whether `self`'s place is `other`'s or holds it.
    fn covers(&self, other: &CaptureRead) -> bool {
        self.root == other.root && other.path.starts_with(&self.path)
    }
}

impl<'a> BodyChecker<'a> {
    /// Whether `local` is outside the innermost closure literal being checked: a capture.
    pub(crate) fn is_capture(&self, local: LocalId) -> bool {
        self.frames.last().is_some_and(|f| local.0 < f.first_local)
    }

    /// The message for changing (or moving) the captured variable `name`.
    pub(crate) fn captured_message(name: &str) -> String {
        format!("`{name}` is captured by this closure, so it can't change here; return a value instead, or use a `for` loop")
    }

    /// After a name or a field access `e` is checked: a name of an outside local starts a read,
    /// and a field of a struct (or of a `match` binding's variant) extends the read of its base.
    pub(crate) fn note_capture(&mut self, e: &Expr) {
        if self.frames.is_empty() {
            return;
        }
        match &e.kind {
            ExprKind::Ident(_) => {
                if let Some(&Res::Local(l)) = self.t.res.get(&e.id) {
                    if self.is_capture(l) {
                        self.start_read(e.id, l, e.span);
                    }
                }
            }
            ExprKind::Field { base, name, .. } => {
                let Some(&field) = self.t.field_index.get(&e.id) else { return };
                if self.t.swizzles.contains_key(&e.id) || !matches!(self.t.ty(base.id), Ty::Struct(_) | Ty::Variant(..)) {
                    return;
                }
                // A `match` binding's name is checked as the base of its fields only, not as an
                // expression of its own (see `check_field_base`).
                if let (Some(&Res::Local(l)), Ty::Variant(..)) = (self.t.res.get(&base.id), self.t.ty(base.id)) {
                    let open = self.frames.last().is_some_and(|f| f.reads.at.contains_key(&base.id));
                    if self.is_capture(l) && !open {
                        self.start_read(base.id, l, base.span);
                    }
                }
                self.extend_read(base.id, e.id, e.span, field, name, self.t.ty(e.id));
            }
            _ => {}
        }
    }

    /// `b.on(args)`, a call of the function field `on`: when `b` is a capture's place, the call
    /// reads the field.
    pub(crate) fn note_field_call_capture(&mut self, e: &Expr, receiver: &Expr, field: u32, name: &str, ty: Ty) {
        self.extend_read(receiver.id, e.id, e.span, field, name, ty);
    }

    /// `f(args)` by the name of `local`, a function value.
    pub(crate) fn note_local_call_capture(&mut self, e: &Expr, local: LocalId, span: Span) {
        if self.is_capture(local) {
            self.start_read(e.id, local, span);
        }
    }

    fn start_read(&mut self, id: ExprId, root: LocalId, span: Span) {
        let ty = self.t.local(root).ty;
        let reads = &mut self.frames.last_mut().expect("a closure is being checked").reads;
        reads.at.insert(id, reads.list.len());
        reads.list.push(CaptureRead { expr: Some(id), root, path: Vec::new(), names: Vec::new(), ty, span });
    }

    /// The read that is `base` goes on into its field `field` (named `name`, of type `ty`): it is
    /// the expression `id` now.
    fn extend_read(&mut self, base: ExprId, id: ExprId, span: Span, field: u32, name: &str, ty: Ty) {
        let Some(frame) = self.frames.last_mut() else { return };
        let Some(i) = frame.reads.at.remove(&base) else { return };
        let r = &mut frame.reads.list[i];
        r.expr = Some(id);
        r.path.push(field);
        r.names.push(name.to_string());
        r.ty = ty;
        r.span = span;
        frame.reads.at.insert(id, i);
    }

    /// The place a local reads, with a view (a loop item, a `match` binding, a `let ... else`
    /// view) expanded to its source: the variable, and the steps into it.
    fn view_source(&self, l: LocalId) -> (LocalId, Vec<Step>) {
        if let Some(x) = self.for_in_lists.iter().rev().find(|x| x.item == l) {
            let mut path = x.path.clone();
            path.push(None);
            return (x.root, path);
        }
        if let Some(p) = self.removing.views.get(&l) {
            return p.clone();
        }
        (self.t.local(l).alias.unwrap_or(l), Vec::new())
    }

    /// The end of closure literal `id`, whose frame was just closed with these reads: its
    /// captures. Records each read in `capture_reads`, reports the captures that aren't allowed,
    /// and hands the ones from outside the enclosing closure (if any) on to it.
    pub(crate) fn finish_captures(&mut self, id: ExprId, reads: Reads) -> Vec<Capture> {
        let list = reads.list;
        // A read is a capture unless another covers it (an equal one counts at its first read).
        let firsts: Vec<usize> = (0..list.len())
            .filter(|&i| !list.iter().enumerate().any(|(j, q)| j != i && q.covers(&list[i]) && (q.path.len() < list[i].path.len() || j < i)))
            .collect();
        for r in &list {
            let Some(e) = r.expr else { continue };
            let k = firsts.iter().position(|&c| list[c].covers(r)).expect("every read is covered by a capture");
            let rest = r.path[list[firsts[k]].path.len()..].to_vec();
            self.t.capture_reads.insert(e, (id, k as u32, rest));
        }
        let mut captures = Vec::with_capacity(firsts.len());
        for &c in &firsts {
            let r = &list[c];
            let written = std::iter::once(self.t.local(r.root).name.clone()).chain(r.names.iter().cloned()).collect::<Vec<_>>().join(".");
            if r.expr.is_some() {
                self.check_capture(r.ty, &written, r.span);
            }
            let (source_root, mut source_path) = self.view_source(r.root);
            source_path.extend(r.names.iter().cloned().map(Some));
            captures.push(Capture { root: r.root, path: r.path.clone(), ty: r.ty, source_root, source_path, span: r.span });
        }
        // The enclosing closure copies these out of its own environment to make this one.
        for (&c, cap) in firsts.iter().zip(&captures) {
            if self.is_capture(cap.root) {
                let read = CaptureRead { expr: None, ..list[c].clone() };
                self.frames.last_mut().expect("an enclosing closure").reads.list.push(read);
            }
        }
        captures
    }

    /// A capture, written `written`, of type `ty`: it can't own an arena or a GPU object, and it
    /// can't be a whole struct or enum value that holds a function value anywhere in its fields
    /// (`self` holding `cb`, or `handlers: list[(int)]`). A function value itself, or a list,
    /// array, map or optional of them, is fine: storing the closure into one of those is a
    /// growth the program asks for, not an accident of capturing `self`.
    fn check_capture(&mut self, ty: Ty, written: &str, span: Span) {
        if self.t.is_owning(ty) {
            let what = if self.t.holds_arena(ty) { "an Arena" } else { "a GPU object" };
            self.error(span, format!("a closure can't capture `{written}`: it owns {what}; pass it as a parameter instead"));
        } else if matches!(ty, Ty::Struct(_) | Ty::Enum(_) | Ty::Variant(..)) && self.t.holds_fn(ty) {
            let msg = format!("this closure would copy all of `{written}`, including its function fields; copy what it needs into a local first");
            self.error(span, msg);
        }
    }
}
