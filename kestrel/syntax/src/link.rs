//! Links several parsed files (modules) into one program. Items of every
//! module except the entry are renamed `module.name`, and each module's
//! references are rewritten to what they mean there: its own items, names
//! it imported with `use { a from m }`, and `m.a` for a module it imported
//! whole with `use m`.

use std::collections::HashMap;

use crate::ast::*;
use crate::diag::Diagnostic;
use crate::span::Span;

pub struct Module {
    /// The file name without `.kes`.
    pub name: String,
    pub program: Program,
}

struct Export {
    qualified: String,
    is_pub: bool,
}

enum Scoped {
    Item(String),
    Module(usize),
}

/// Links `modules`, the entry file first. Their ids must already be
/// distinct (see `parse_at`).
pub fn link(modules: Vec<Module>) -> (Program, Vec<Diagnostic>) {
    let index: HashMap<String, usize> = modules.iter().enumerate().map(|(i, m)| (m.name.clone(), i)).collect();
    let exports: Vec<HashMap<String, Export>> = modules
        .iter()
        .enumerate()
        .map(|(i, m)| {
            let qualify = |name: &str| if i == 0 { name.to_string() } else { format!("{}.{name}", m.name) };
            m.program
                .items
                .iter()
                .filter_map(|item| match item {
                    Item::Fn(f) => Some((f.name.clone(), f.is_pub)),
                    Item::Struct(s) => Some((s.name.clone(), s.is_pub)),
                    Item::Enum(e) => Some((e.name.clone(), e.is_pub)),
                    Item::Const(c) => Some((c.bind.name.clone(), c.is_pub)),
                    Item::Use(_) => None,
                })
                .map(|(name, is_pub)| (name.clone(), Export { qualified: qualify(&name), is_pub }))
                .collect()
        })
        .collect();
    let names: Vec<String> = modules.iter().map(|m| m.name.clone()).collect();
    let (mut expr_count, mut bind_count) = (0, 0);
    let mut items = Vec::new();
    let mut diags = Vec::new();
    for (i, m) in modules.into_iter().enumerate() {
        expr_count = expr_count.max(m.program.expr_count);
        bind_count = bind_count.max(m.program.bind_count);
        let mut rw = Rewriter { scope: HashMap::new(), exports: &exports, names: &names, diags: Vec::new() };
        for (name, e) in &exports[i] {
            rw.scope.insert(name.clone(), Scoped::Item(e.qualified.clone()));
        }
        for item in &m.program.items {
            if let Item::Use(u) = item {
                rw.import(u, &index);
            }
        }
        for mut item in m.program.items {
            if !matches!(item, Item::Use(_)) {
                rw.item(&mut item);
                items.push(item);
            }
        }
        diags.append(&mut rw.diags);
    }
    (Program { items, expr_count, bind_count, builtin_ranges: Vec::new() }, diags)
}

struct Rewriter<'a> {
    scope: HashMap<String, Scoped>,
    exports: &'a [HashMap<String, Export>],
    names: &'a [String],
    diags: Vec<Diagnostic>,
}

impl Rewriter<'_> {
    fn error(&mut self, span: Span, msg: String) {
        self.diags.push(Diagnostic::error(span, msg));
    }

    fn import(&mut self, u: &UseDecl, index: &HashMap<String, usize>) {
        for c in &u.clauses {
            let Some(&m) = index.get(&c.module) else {
                let msg = format!("can't find module `{0}`: there's no `{0}.kes` next to this file", c.module);
                self.error(c.module_span, msg);
                continue;
            };
            if c.names.is_empty() {
                if self.scope.contains_key(&c.module) {
                    self.error(c.module_span, format!("`{}` is already defined here", c.module));
                } else {
                    self.scope.insert(c.module.clone(), Scoped::Module(m));
                }
                continue;
            }
            for (name, span) in &c.names {
                let Some(q) = self.exported(m, name, *span) else { continue };
                if self.scope.contains_key(name) {
                    self.error(*span, format!("`{name}` is already defined here"));
                } else {
                    self.scope.insert(name.clone(), Scoped::Item(q));
                }
            }
        }
    }

    /// The qualified name of `pub` item `name` of module `m`.
    fn exported(&mut self, m: usize, name: &str, span: Span) -> Option<String> {
        let module = &self.names[m];
        match self.exports[m].get(name) {
            Some(e) if e.is_pub => Some(e.qualified.clone()),
            Some(_) => {
                let msg = format!("`{name}` isn't `pub` in `{module}`");
                self.error(span, msg);
                None
            }
            None => {
                let msg = format!("`{module}` has no `{name}`");
                self.error(span, msg);
                None
            }
        }
    }

    fn qualified(&self, name: &str) -> Option<String> {
        match self.scope.get(name) {
            Some(Scoped::Item(q)) => Some(q.clone()),
            _ => None,
        }
    }

    /// The module `e` names, when it's a bare `use m` alias.
    fn module(&self, e: &Expr) -> Option<usize> {
        let ExprKind::Ident(n) = &e.kind else { return None };
        match self.scope.get(n) {
            Some(Scoped::Module(m)) => Some(*m),
            _ => None,
        }
    }

    fn item(&mut self, item: &mut Item) {
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
                s.methods.iter_mut().for_each(|m| self.func(m));
            }
            Item::Enum(e) => {
                e.name = self.qualified(&e.name).unwrap_or_else(|| e.name.clone());
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

    fn func(&mut self, f: &mut FnDecl) {
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
    }

    fn ty(&mut self, t: &mut TypeExpr) {
        match &mut t.kind {
            TypeKind::Named { name, args } => {
                if let Some(q) = self.qualified(name) {
                    *name = q;
                }
                args.iter_mut().for_each(|a| self.ty(a));
            }
            TypeKind::Array(inner) | TypeKind::Optional(inner) => self.ty(inner),
        }
    }

    /// A local can't take the name of an import (the checker only sees the
    /// qualified names, so it couldn't tell).
    fn binding(&mut self, b: &Binding) {
        let clash = match self.scope.get(&b.name) {
            Some(Scoped::Module(_)) => true,
            Some(Scoped::Item(q)) => *q != b.name,
            None => false,
        };
        if clash {
            self.error(b.span, format!("`{}` is already defined: it's imported; pick another name", b.name));
        }
    }

    fn block(&mut self, b: &mut Block) {
        b.stmts.iter_mut().for_each(|s| self.stmt(s));
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
                self.binding(var);
                self.expr(start);
                self.expr(end);
                self.block(body);
            }
            Stmt::ForIn { index, var, iter, body, .. } => {
                if let Some(i) = index {
                    self.binding(i);
                }
                self.binding(var);
                self.expr(iter);
                self.block(body);
            }
            Stmt::Arena { body, .. } => self.block(body),
            Stmt::In { arena, body, .. } => {
                self.expr(arena);
                self.block(body);
            }
            Stmt::Return { value: Some(v), .. } | Stmt::Expr(v) => self.expr(v),
            Stmt::Return { value: None, .. } | Stmt::Break(_) | Stmt::Continue(_) => {}
        }
    }

    fn args(&mut self, args: &mut [Arg]) {
        args.iter_mut().for_each(|a| self.expr(&mut a.value));
    }

    fn expr(&mut self, e: &mut Expr) {
        match &mut e.kind {
            ExprKind::Ident(name) => match self.scope.get(name.as_str()) {
                Some(Scoped::Item(q)) => *name = q.clone(),
                Some(Scoped::Module(_)) => {
                    let msg = format!("`{name}` is a module; use one of its items, like `{name}.something`");
                    self.error(e.span, msg);
                }
                None => {}
            },
            ExprKind::Call { name, args, .. } => {
                if let Some(q) = self.qualified(name) {
                    *name = q;
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
                    if let Pattern::Variant { binding: Some(b), .. } = &a.pattern {
                        self.binding(b);
                    }
                    self.block(&mut a.body);
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
