//! Owning temporaries. An owning value (one holding an `Arena` or a `GpuHandle`) that is made
//! and not stored, like `draw.texture(draw.load_texture(path), ...)`, has no variable to drop it.
//! Each of these is bound to a hidden `let` of the block it is made in, so the block's end drops
//! it like it drops the block's own `let`s, and the drop is skipped when the value was moved
//! out (a moved place is zeroed). Values that are stored (`let`, assignment, `list.add`, a field
//! of a construction, `return`) move as before, and non-owning values get no temporary.

use kestrel_check::Ty;

use super::helpers::owning;
use super::{kty, FnLowerer};
use crate::ir::*;

impl<'a> FnLowerer<'a> {
    /// Whether `v` is an owning value that nothing holds yet, so nothing else will drop it.
    pub(crate) fn is_new_owning(&self, v: &Expr) -> bool {
        owning(v.ty, self.t) && self.builds_new(v)
    }

    /// Whether `v` builds its value, rather than reading one a place or an owner still has.
    fn builds_new(&self, v: &Expr) -> bool {
        match &v.kind {
            ExprKind::Load(_) | ExprKind::FieldOf(..) => false,
            // The result of a native made in a scratch arena, or a value taken out of a held temporary.
            ExprKind::Move(p) => self.held.contains(&p.root),
            ExprKind::Seq(_, inner) | ExprKind::Some(inner) => self.builds_new(inner),
            ExprKind::If { then_val, else_val, .. } => [then_val, else_val].into_iter().flatten().all(|v| self.builds_new(v)),
            _ => true,
        }
    }

    /// Binds the new owning value `v` to a hidden `let` of the current block and returns it.
    fn keep(&mut self, v: Expr, pre: &mut Vec<Stmt>) -> LocalIdx {
        let tmp = self.temp(v.ty);
        pre.push(Stmt::Let(tmp, v));
        self.declared(tmp);
        self.held.insert(tmp);
        tmp
    }

    /// `v` where nothing stores it (an argument to a read-only parameter, a printed value):
    /// held until the block ends when it is a new owning value, else unchanged.
    pub(crate) fn held(&mut self, v: Expr) -> Expr {
        if !self.is_new_owning(&v) {
            return v;
        }
        let (ty, line) = (v.ty, v.line);
        let mut pre = Vec::new();
        let tmp = self.keep(v, &mut pre);
        Expr { kind: ExprKind::Seq(pre, Box::new(Self::load(tmp, ty, line))), ty, line }
    }

    /// Like `spill`, and also holds `v` until the block ends when it is a new owning value.
    pub(crate) fn spill_held(&mut self, v: Expr, pre: &mut Vec<Stmt>) -> Expr {
        if !self.is_new_owning(&v) {
            return self.spill(v, pre);
        }
        let (ty, line) = (v.ty, v.line);
        let tmp = self.keep(v, pre);
        Self::load(tmp, ty, line)
    }

    /// A place held in a temporary for `place_or_temp`: the new owning value `v` is held until
    /// the block ends; anything else just goes in a temporary.
    pub(crate) fn temp_place(&mut self, v: Expr, pre: &mut Vec<Stmt>) -> Place {
        if self.is_new_owning(&v) {
            return Place::local(self.keep(v, pre));
        }
        let tmp = self.temp(v.ty);
        pre.push(Stmt::Let(tmp, v));
        Place::local(tmp)
    }

    /// `base.field` where `base` is not a place. A new owning `base` is held, and its field is
    /// read from the held value, like a field of a variable (so a stored copy of the field is
    /// made, or moved out of it, before the block drops the rest).
    pub(crate) fn field_of(&mut self, base: Expr, field: u32, ty: KTy, line: u32) -> ExprKind {
        if !self.is_new_owning(&base) {
            return ExprKind::FieldOf(Box::new(base), field);
        }
        let mut pre = Vec::new();
        let mut place = Place::local(self.keep(base, &mut pre));
        place.path.push(Proj::Field(field));
        Self::with_pre(pre, ExprKind::Load(place), ty, line)
    }

    /// A call whose result is discarded. If the result is an owning value (`ret` is the callee's
    /// result type), it is held until the block ends instead of being lost.
    pub(crate) fn call_stmt(&mut self, f: FuncIdx, args: Vec<Arg>, ret: Ty, line: u32, out: &mut Vec<Stmt>) {
        if self.t.is_owning(ret) {
            let call = Expr { kind: ExprKind::Call(f, args), ty: kty(ret), line };
            self.keep(call, out);
        } else {
            out.push(Stmt::Call(f, args));
        }
    }

    /// `v` about to be stored: a read of a held temporary's owning part becomes a move out of
    /// it, so dropping the temporary skips what was taken.
    pub(crate) fn release(&self, v: Expr) -> Expr {
        let Expr { kind, ty, line } = v;
        let kind = match kind {
            ExprKind::Load(p) if owning(ty, self.t) && self.held.contains(&p.root) => ExprKind::Move(p),
            ExprKind::Seq(stmts, inner) => ExprKind::Seq(stmts, Box::new(self.release(*inner))),
            ExprKind::Some(inner) => ExprKind::Some(Box::new(self.release(*inner))),
            ExprKind::If { cond, then_body, then_val, else_body, else_val } => ExprKind::If {
                cond,
                then_body,
                then_val: then_val.map(|v| Box::new(self.release(*v))),
                else_body,
                else_val: else_val.map(|v| Box::new(self.release(*v))),
            },
            other => other,
        };
        Expr { kind, ty, line }
    }

    fn load(tmp: LocalIdx, ty: KTy, line: u32) -> Expr {
        Expr { kind: ExprKind::Load(Place::local(tmp)), ty, line }
    }
}
