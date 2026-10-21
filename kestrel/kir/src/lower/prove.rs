//! `@prove` in lowering: a `ContractSite` for each annotated function and method, and one for each
//! implementation of an annotated interface method (see `iface::implementations`); an annotated
//! loop's statements wrapped in one `Stmt::Prove`, its site made the first time it is lowered.
//! Where lowering moves statements into a function of their own (a `par for` body, a closure's
//! code, the constants' initializer), `repoint` gives each loop site in them that function.
//!
//! A program with no `@prove` does none of this: the index of annotations is empty.

use std::collections::HashMap;

use kestrel_check::{FnId, Tables};
use kestrel_syntax::ast::{self, Program};
use kestrel_syntax::Span;

use super::helpers::Helpers;
use super::{iface, FnLowerer};
use crate::ir::*;
use crate::visit::{walk, Node};

/// The `@prove`s of a program by the span of what each is on (an index into `Program::proves`),
/// and the site made for each annotated loop lowered so far.
#[derive(Default)]
pub(crate) struct Proves {
    by_target: HashMap<Span, usize>,
    loops: HashMap<usize, u32>,
    pub(crate) sites: Vec<ContractSite>,
}

impl Proves {
    /// The index of `program`'s annotations, and a site for each annotated function and method and
    /// each implementation of an annotated interface method, in the order of `t.fns`.
    pub(crate) fn new(program: &Program, t: &Tables) -> Self {
        let mut p = Proves::default();
        if program.proves.is_empty() {
            return p;
        }
        p.by_target = program.proves.iter().enumerate().map(|(i, a)| (a.target, i)).collect();
        for (i, info) in t.fns.iter().enumerate() {
            let decl = info.decl(program);
            let Some(&k) = p.by_target.get(&decl.span) else { continue };
            let contracts = &program.proves[k].contracts;
            if !info.is_abstract {
                let target = ProveTarget::Func(FuncIdx(i as u32));
                p.sites.push(ContractSite { target, contracts: contracts.clone(), required_by: None });
                continue;
            }
            for imp in iface::implementations(t, FnId(i as u32), &decl.name) {
                let at = t.func(imp).decl(program).name_span;
                p.sites.push(ContractSite {
                    target: ProveTarget::Func(FuncIdx(imp.0)),
                    contracts: contracts.iter().map(|&(c, _)| (c, at)).collect(),
                    required_by: Some(format!("`{}`", info.name)),
                });
            }
        }
        p
    }
}

impl Helpers {
    /// Gives each loop site whose `Prove` block is in `body` the function `to`, where `body` now is.
    pub(crate) fn repoint(&mut self, body: &[Stmt], to: FuncIdx) {
        if self.proves.loops.is_empty() {
            return;
        }
        let sites = &mut self.proves.sites;
        walk(body, &mut |n| {
            if let Node::Stmt(Stmt::Prove { id, .. }) = n {
                if let ProveTarget::Loop { func, .. } = &mut sites[*id as usize].target {
                    *func = to;
                }
            }
        });
    }
}

/// Panics unless every loop site's function holds the site's `Prove` block: lowering moved one
/// without `repoint`.
pub(crate) fn check_homes(m: &Module) {
    for (id, site) in m.contracts.iter().enumerate() {
        let ProveTarget::Loop { func, line } = site.target else { continue };
        let mut found = false;
        if let Some(f) = m.funcs.get(func.0 as usize) {
            walk(&f.body, &mut |n| found |= matches!(n, Node::Stmt(Stmt::Prove { id: i, .. }) if *i as usize == id));
        }
        assert!(found, "lowering lost track of the loop at packed line {line}: function {} doesn't hold its `Prove`", func.0);
    }
}

impl FnLowerer<'_> {
    /// Lowers `s` into one `Stmt::Prove` when it is a loop with a `@prove`; false otherwise.
    pub(crate) fn annotated_loop(&mut self, s: &ast::Stmt, out: &mut Vec<Stmt>) -> bool {
        if self.helpers.proves.by_target.is_empty() || !matches!(s, ast::Stmt::While { .. } | ast::Stmt::ForRange { .. } | ast::Stmt::ForIn { .. }) {
            return false;
        }
        let Some(&k) = self.helpers.proves.by_target.get(&s.span()) else { return false };
        let id = match self.helpers.proves.loops.get(&k) {
            Some(&id) => id,
            None => {
                let id = u32::try_from(self.helpers.proves.sites.len()).expect("fewer than 2^32 contracts");
                // A closure's code and the constants' initializer are repointed when they are made.
                let func = self.fn_id.map_or(FuncIdx(u32::MAX), |f| FuncIdx(f.0));
                let target = ProveTarget::Loop { func, line: self.line(s.span()) };
                let contracts = self.program.proves[k].contracts.clone();
                self.helpers.proves.sites.push(ContractSite { target, contracts, required_by: None });
                self.helpers.proves.loops.insert(k, id);
                id
            }
        };
        let mut body = Vec::new();
        self.plain_stmt(s, &mut body);
        out.push(Stmt::Prove { id, at: 0, body });
        true
    }
}
