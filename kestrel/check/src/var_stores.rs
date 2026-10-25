//! Stores into `var` parameters, checked where the function is called (P38).
//!
//! A body that stores memory from the caller's current arena into one of its own `var` parameters
//! records a **direct store** instead of an error (`regions::check_store_region`). Each direct call
//! records what its caller would need if the callee turns out to store into a parameter: an
//! error (the place passed doesn't hold the caller's memory) or a store of the caller's own (the
//! place passed is one of the caller's `var` parameters). A function named as a value records
//! where. Nothing is decided while bodies are checked: `resolve` works the summaries out to a
//! fixpoint after the last body, so declaration order and recursion don't matter.

use std::collections::HashSet;

use kestrel_syntax::ast::ExprId;
use kestrel_syntax::{Diagnostic, Span};

use crate::tables::{FnId, Tables};

/// A function's `var` parameter, by its index in the function's parameters (`self` is 0).
type VarParam = (FnId, u32);

/// What `resolve` works from. Public only so other crates can build a `Tables` with
/// `..Tables::default()`; its contents are the checker's own.
#[derive(Default, Clone, Debug)]
pub struct VarStores {
    /// Bodies that store caller memory into their own `var` parameter.
    pub(crate) direct: Vec<VarParam>,
    pub(crate) calls: Vec<PendingCall>,
    /// Functions named as values, and where.
    pub(crate) values: Vec<(FnId, Span)>,
}

/// A direct call's `var` argument for parameter `param` of `callee`, and what follows if the callee
/// stores into it: the caller stores into its own parameter (`caller`), or the call is refused.
#[derive(Clone, Debug)]
pub(crate) struct PendingCall {
    /// The call expression, so a call recorded twice is reported once.
    pub(crate) call: ExprId,
    pub(crate) callee: FnId,
    pub(crate) param: u32,
    pub(crate) caller: Option<VarParam>,
    pub(crate) refused: Option<Diagnostic>,
}

/// Works out every function's summary (its `var` parameters that may receive caller memory) and
/// reports the calls and values that the summaries refuse. Never panics on a broken program:
/// every index is checked (the language server checks programs with errors).
pub(crate) fn resolve(t: &mut Tables, diags: &mut Vec<Diagnostic>) {
    let stores = std::mem::take(&mut t.var_stores);
    let edges = iface_edges(t);
    let mut summary: HashSet<VarParam> = stores.direct.iter().copied().collect();
    loop {
        let mut changed = false;
        for c in &stores.calls {
            if let (Some(caller), true) = (c.caller, summary.contains(&(c.callee, c.param))) {
                changed |= summary.insert(caller);
            }
        }
        for &(from, to) in &edges {
            if summary.contains(&from) {
                changed |= summary.insert(to);
            }
        }
        if !changed {
            break;
        }
    }
    let mut reported: HashSet<(ExprId, u32)> = HashSet::new();
    for c in &stores.calls {
        let Some(d) = &c.refused else { continue };
        if summary.contains(&(c.callee, c.param)) && reported.insert((c.call, c.param)) {
            diags.push(d.clone());
        }
    }
    for &(fid, span) in &stores.values {
        let Some(info) = t.fns.get(fid.0 as usize) else { continue };
        let Some(p) = (0..info.params.len() as u32).find(|&i| summary.contains(&(fid, i))) else { continue };
        let (f, p) = (&info.name, &info.params[p as usize].name);
        let msg = format!("`{f}` stores memory from the caller's current arena in its `var` parameter `{p}`, so it can't be used as a value; call it by name instead");
        diags.push(Diagnostic::error(span, msg).with_help(format!("a call by name checks that what you pass for `{p}` lives as long as that memory")));
    }
}

/// An interface method stores into a `var` parameter when any method that implements it does:
/// edges from each implementation's `var` parameter to the abstract method's. The implementations
/// are the methods of that name of every struct and enum listing the interface (an instance lists
/// its declaration's interfaces and shares its methods, so it adds nothing new).
fn iface_edges(t: &Tables) -> Vec<(VarParam, VarParam)> {
    let mut edges = Vec::new();
    let types = t.structs.iter().map(|s| (&s.ifaces, &s.methods)).chain(t.enums.iter().map(|e| (&e.ifaces, &e.methods)));
    for (ifaces, methods) in types {
        for iface in ifaces.iter().filter_map(|i| t.ifaces.get(i.0 as usize)) {
            for &abs in &iface.methods {
                let Some(abs_info) = t.fns.get(abs.0 as usize) else { continue };
                let name = abs_info.name.rsplit('.').next().unwrap_or_default();
                let Some(imp_info) = methods.get(name).and_then(|m| t.fns.get(m.0 as usize).map(|i| (*m, i))) else { continue };
                let (imp, imp_info) = imp_info;
                // A signature that doesn't fit its interface is already an error; match what lines up.
                for (k, (a, b)) in abs_info.params.iter().zip(&imp_info.params).enumerate() {
                    if a.is_var && b.is_var {
                        edges.push(((imp, k as u32), (abs, k as u32)));
                    }
                }
            }
        }
    }
    edges
}
