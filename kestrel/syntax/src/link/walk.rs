//! The walk over one module's items that rewrites its names (see `link`), keeping track of
//! the locals in scope.

use super::{Rewriter, Scoped};
use crate::ast::*;

impl Rewriter<'_> {
    pub(super) fn item(&mut self, item: &mut Item) {
        match item {
            Item::Fn(f) => {
                f.name = self.qualified(&f.name).unwrap_or_else(|| f.name.clone());
                self.func(f);
            }
            Item::Struct(s) => {
                s.name = self.qualified(&s.name).unwrap_or_else(|| s.name.clone());
                for f in &mut s.fields {
                    self.ty(&mut f.ty);
                    if let Some(d) = &mut f.default {
                        self.expr(d);
                    }
                }
                self.iface_names(&mut s.ifaces);
                s.methods.iter_mut().for_each(|m| self.func(m));
            }
            Item::Interface(i) => {
                i.name = self.qualified(&i.name).unwrap_or_else(|| i.name.clone());
                i.methods.iter_mut().for_each(|m| self.func(m));
            }
            Item::Enum(e) => {
                e.name = self.qualified(&e.name).unwrap_or_else(|| e.name.clone());
                self.iface_names(&mut e.ifaces);
                for v in &mut e.variants {
                    v.fields.iter_mut().for_each(|f| self.ty(&mut f.ty));
                }
                e.methods.iter_mut().for_each(|m| self.func(m));
            }
            Item::Const(c) => {
                c.bind.name = self.qualified(&c.bind.name).unwrap_or_else(|| c.bind.name.clone());
                if let Some(t) = &mut c.ty {
                    self.ty(t);
                }
                self.expr(&mut c.value);
            }
            Item::Use(_) => {}
        }
    }

    /// The interfaces in `struct S: A, B` are named like types.
    fn iface_names(&mut self, ifaces: &mut [(String, crate::span::Span)]) {
        for (name, span) in ifaces {
            match self.qualified(name) {
                Some(q) => *name = q,
                None => self.unseen(&name.clone(), *span, "type", false),
            }
        }
    }

    fn func(&mut self, f: &mut FnDecl) {
        let mark = self.locals.len();
        for p in &mut f.params {
            // A native has no body, so its parameter names are only labels and hide nothing:
            // `draw.text(text: string, ...)` may name its parameter `text`.
            if !p.is_self && !f.is_native {
                self.binding(&p.bind);
            }
            self.ty(&mut p.ty);
        }
        if let Some(r) = &mut f.ret {
            self.ty(r);
        }
        self.block(&mut f.body);
        self.locals.truncate(mark);
    }

    fn ty(&mut self, t: &mut TypeExpr) {
        match &mut t.kind {
            TypeKind::Named { name, args } => {
                match self.qualified(name) {
                    Some(q) => *name = q,
                    None => self.unseen(&name.clone(), t.span, "type", false),
                }
                args.iter_mut().for_each(|a| self.ty(a));
            }
            TypeKind::Array(inner) | TypeKind::Optional(inner) => self.ty(inner),
            TypeKind::Fn { params, ret, .. } => {
                params.iter_mut().for_each(|p| self.ty(&mut p.ty));
                if let Some(r) = ret {
                    self.ty(r);
                }
            }
        }
    }

    /// A block's own locals go out of scope at its end.
    fn block(&mut self, b: &mut Block) {
        let mark = self.locals.len();
        b.stmts.iter_mut().for_each(|s| self.stmt(s));
        self.locals.truncate(mark);
    }

    fn stmt(&mut self, s: &mut Stmt) {
        match s {
            Stmt::Let { bind, ty, value, else_block, .. } => {
                self.expr(value);
                if let Some(t) = ty {
                    self.ty(t);
                }
                if let Some(b) = else_block {
                    self.block(b);
                }
                self.binding(bind);
            }
            Stmt::Assign { target, value, .. } => {
                self.expr(target);
                self.expr(value);
            }
            Stmt::While { cond, body, .. } => {
                self.expr(cond);
                self.block(body);
            }
            Stmt::ForRange { var, start, end, body, .. } => {
                self.expr(start);
                self.expr(end);
                let mark = self.locals.len();
                self.binding(var);
                self.block(body);
                self.locals.truncate(mark);
            }
            Stmt::ForIn { index, var, iter, body, .. } => {
                self.expr(iter);
                let mark = self.locals.len();
                if let Some(i) = index {
                    self.binding(i);
                }
                self.binding(var);
                self.block(body);
                self.locals.truncate(mark);
            }
            Stmt::Arena { body, .. } => self.block(body),
            Stmt::In { arena, body, .. } => {
                self.expr(arena);
                self.block(body);
            }
            Stmt::Return { value: Some(v), .. } | Stmt::Expr(v) => self.expr(v),
            Stmt::Return { value: None, .. } | Stmt::Break(_) | Stmt::Continue(_) | Stmt::Remove { .. } => {}
        }
    }

    fn args(&mut self, args: &mut [Arg]) {
        args.iter_mut().for_each(|a| self.expr(&mut a.value));
    }

    fn expr(&mut self, e: &mut Expr) {
        match &mut e.kind {
            ExprKind::Ident(name) => match self.scope.get(name.as_str()) {
                Some(Scoped::Own(q) | Scoped::Item(q)) => *name = q.clone(),
                Some(Scoped::Module(_)) => {
                    let msg = format!("`{name}` is a module; use one of its items, like `{name}.something`");
                    self.error(e.span, msg);
                }
                None => self.unseen(&name.clone(), e.span, "name", true),
            },
            ExprKind::Call { name, name_span, args } => {
                match self.qualified(name) {
                    Some(q) => *name = q,
                    None => self.unseen(&name.clone(), *name_span, "function", false),
                }
                self.args(args);
            }
            ExprKind::MethodCall { receiver, name, name_span, args } => match self.module(receiver) {
                // `m.f(...)` calls `f` from module `m`.
                Some(m) => {
                    let (name, name_span, mut args) = (name.clone(), *name_span, std::mem::take(args));
                    self.args(&mut args);
                    let name = self.exported(m, &name, name_span).unwrap_or(name);
                    e.kind = ExprKind::Call { name, name_span, args };
                }
                None => {
                    self.expr(receiver);
                    self.args(args);
                }
            },
            ExprKind::Field { base, name, name_span } => match self.module(base) {
                // `m.LIMIT`, or `m.Shape` in `m.Shape.Circle(...)`.
                Some(m) => {
                    let (name, name_span) = (name.clone(), *name_span);
                    let name = self.exported(m, &name, name_span).unwrap_or(name);
                    e.kind = ExprKind::Ident(name);
                }
                None => self.expr(base),
            },
            ExprKind::CallValue { callee, args } => {
                self.expr(callee);
                self.args(args);
            }
            ExprKind::Closure(c) => {
                let mark = self.locals.len();
                for p in &mut c.params {
                    self.binding(&p.bind);
                    if let Some(t) = &mut p.ty {
                        self.ty(t);
                    }
                }
                if let Some(r) = &mut c.ret {
                    self.ty(r);
                }
                self.block(&mut c.body);
                self.locals.truncate(mark);
            }
            ExprKind::Unary { expr, .. } => self.expr(expr),
            ExprKind::Binary { lhs, rhs, .. } => {
                self.expr(lhs);
                self.expr(rhs);
            }
            ExprKind::If { cond, then_block, else_block } => {
                self.expr(cond);
                self.block(then_block);
                if let Some(b) = else_block {
                    self.block(b);
                }
            }
            ExprKind::Match { scrutinee, arms } => {
                self.expr(scrutinee);
                for a in arms {
                    let mark = self.locals.len();
                    if let Pattern::Variant { binding: Some(b), .. } = &a.pattern {
                        self.binding(b);
                    }
                    self.block(&mut a.body);
                    self.locals.truncate(mark);
                }
            }
            ExprKind::Index { base, index } => {
                self.expr(base);
                self.expr(index);
            }
            ExprKind::ArrayLit(items) => items.iter_mut().for_each(|i| self.expr(i)),
            ExprKind::MapLit(entries) => {
                for (k, v) in entries {
                    self.expr(k);
                    self.expr(v);
                }
            }
            ExprKind::TypeCall { type_args, args, .. } => {
                type_args.iter_mut().for_each(|t| self.ty(t));
                self.args(args);
            }
            ExprKind::Interp(parts) => {
                for p in parts {
                    if let InterpPart::Expr(x) = p {
                        self.expr(x);
                    }
                }
            }
            ExprKind::Int(_) | ExprKind::Float(_) | ExprKind::Bool(_) | ExprKind::None | ExprKind::Str(_) => {}
        }
    }
}
