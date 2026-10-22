//! Links several parsed files (modules) into one program. Items of every
//! module except the entry are renamed `module.name`, and each module's
//! references are rewritten to what they mean there: its own items, names
//! it imported with `use { a from m }`, and `m.a` for a module it imported
//! whole with `use m`.
//!
//! Each module sees only those names, so this is also where a local is checked against them:
//! a local can't take the name of one of its module's items or imports. (The checker sees the
//! linked program, where another module's items, the entry file's included, are all global.)

mod walk;

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

/// What a name means inside one module.
enum Scoped {
    /// One of the module's own items, by its qualified name.
    Own(String),
    /// An item imported with `use { a from m }`, by its qualified name.
    Item(String),
    /// A module imported whole with `use m`.
    Module(usize),
}

/// Links `modules`, the entry file first. Their ids must already be
/// distinct (see `parse_at`). `reserved` says which names no item may take (the checker
/// refuses such an item, so `link` says nothing more about it).
pub fn link(modules: Vec<Module>, reserved: fn(&str) -> bool) -> (Program, Vec<Diagnostic>) {
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
                    Item::Interface(i) => Some((i.name.clone(), i.is_pub)),
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
    let mut proves = Vec::new();
    let mut hex_literals = Vec::new();
    let mut diags = Vec::new();
    for (i, m) in modules.into_iter().enumerate() {
        expr_count = expr_count.max(m.program.expr_count);
        bind_count = bind_count.max(m.program.bind_count);
        proves.extend(m.program.proves);
        hex_literals.extend(m.program.hex_literals);
        let (scope, locals) = (HashMap::new(), Vec::new());
        let mut rw = Rewriter { scope, exports: &exports, names: &names, module: i, locals, reserved, diags: Vec::new() };
        for (name, e) in &exports[i] {
            rw.scope.insert(name.clone(), Scoped::Own(e.qualified.clone()));
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
    (Program { items, expr_count, bind_count, builtin_ranges: Vec::new(), proves, hex_literals }, diags)
}

struct Rewriter<'a> {
    scope: HashMap<String, Scoped>,
    exports: &'a [HashMap<String, Export>],
    names: &'a [String],
    /// The module being rewritten, an index into `names`; 0 is the entry file.
    module: usize,
    /// The locals and parameters in scope at this point of the module's code.
    locals: Vec<String>,
    /// Names no item may take (see `link`).
    reserved: fn(&str) -> bool,
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
            Some(Scoped::Own(q) | Scoped::Item(q)) => Some(q.clone()),
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

    /// A local or parameter comes into scope. It can't take the name of one of the module's
    /// items or imports (the checker only sees the qualified names, so it couldn't tell).
    fn binding(&mut self, b: &Binding) {
        match self.scope.get(&b.name) {
            Some(Scoped::Module(_) | Scoped::Item(_)) => {
                self.error(b.span, format!("`{}` is already defined: it's imported; pick another name", b.name));
            }
            Some(Scoped::Own(_)) => {
                let msg = format!("`{}` is already defined in this file; Kestrel doesn't allow shadowing", b.name);
                self.diags.push(Diagnostic::error(b.span, msg).with_help("pick a different name"));
            }
            None => {}
        }
        self.locals.push(b.name.clone());
    }

    /// A bare name that means nothing in this module. The entry file's items keep their own
    /// names when linked, so without this another module could reach one it never imported.
    /// `local` is whether a local of that name would be what it means (not for a call or a type).
    fn unseen(&mut self, name: &str, span: Span, what: &str, local: bool) {
        let hidden = self.module != 0 && self.exports[0].contains_key(name) && !(self.reserved)(name);
        if hidden && !(local && self.locals.iter().any(|l| l == name)) {
            let (entry, here) = (&self.names[0], &self.names[self.module]);
            let msg = format!("unknown {what} `{name}`: it's in {entry}.kes, which `{here}` can't see");
            let help = "a module sees only its own items and the ones it imports with `use`";
            self.diags.push(Diagnostic::error(span, msg).with_help(help));
        }
    }
}
