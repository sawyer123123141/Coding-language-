//! The type expansion check (G10, `collect` phase 4): a generic struct or enum whose fields need
//! ever bigger copies of it (`struct Tree[T] { kids: list[Tree[list[T]]] }`) would need endless
//! copies. It is Go's "instantiation cycle" check (`types2/monomorph.go`): a node per placeholder;
//! each instance `G[args]` written in a declaration's fields adds an edge from every placeholder in
//! `args[i]` to `G`'s placeholder `i`, growing unless `args[i]` is exactly that placeholder; a
//! strongly connected component with a growing edge inside is endless.
//!
//! Such a declaration is reported once per cycle, at its first declaration in source order, and
//! every declaration of the cycle goes into `generic.endless` before the pending instances are
//! filled, so filling them (fields `Error`) ends (P9). The function graph (Task 5) reuses `Graph`.

use std::collections::{HashMap, HashSet, VecDeque};

use kestrel_syntax::Diagnostic;

use crate::generic::graph::Graph;
use crate::generic::{GenericOwner, ParamId};
use crate::tables::{EnumId, StructId, Tables};
use crate::ty::Ty;

/// An instance written in a declaration's fields: `G[args]` inside `from`.
struct Site {
    from: GenericOwner,
    to: GenericOwner,
    args: Vec<Ty>,
}

/// Phase 4 of `collect`: reports generic structs and enums that would need endless copies and
/// marks them `endless`. Runs before `fill_pending`, over declarations' field types only (the
/// pending instances have no fields yet).
pub(crate) fn check_type_expansion(t: &mut Tables, diags: &mut Vec<Diagnostic>) {
    let mut graph = Graph::default();
    let mut sites: Vec<Site> = Vec::new();
    for (i, s) in t.structs.iter().enumerate() {
        if s.instance.is_none() && !s.params.is_empty() {
            let from = GenericOwner::Struct(StructId(i as u32));
            for f in &s.fields {
                walk(t, f.ty, from, &mut graph, &mut sites);
            }
        }
    }
    for (i, e) in t.enums.iter().enumerate() {
        if e.instance.is_none() && !e.params.is_empty() {
            let from = GenericOwner::Enum(EnumId(i as u32));
            for f in e.variants.iter().flat_map(|v| &v.fields) {
                walk(t, f.ty, from, &mut graph, &mut sites);
            }
        }
    }
    let mut reported: HashSet<GenericOwner> = HashSet::new();
    for cycle in graph.growing_cycles() {
        let owners: HashSet<GenericOwner> = cycle.iter().map(|&p| t.generic.param(p).owner).collect();
        let first = *owners.iter().min_by_key(|&&o| (decl_item(t, o), owner_key(o))).expect("a cycle has placeholders");
        if reported.insert(first) {
            let (msg, help) = message(t, first, &cycle, &owners, &graph, &sites);
            diags.push(Diagnostic::error(decl_span(t, first), msg).with_help(help));
        }
        t.generic.endless.extend(owners);
    }
}

/// Adds the edges of every instance in `ty` (a field type of declaration `from`), arguments included.
fn walk(t: &Tables, ty: Ty, from: GenericOwner, graph: &mut Graph, sites: &mut Vec<Site>) {
    let (to, args) = match ty {
        Ty::Struct(id) => {
            let (def, args) = t.struct_parts(id);
            (GenericOwner::Struct(def), args)
        }
        Ty::Enum(id) | Ty::Variant(id, _) => {
            let (def, args) = t.enum_parts(id);
            (GenericOwner::Enum(def), args)
        }
        Ty::Array(e) | Ty::List(e) | Ty::Optional(e) => return walk(t, t.elem(e), from, graph, sites),
        Ty::Map(k, v) => {
            walk(t, t.elem(k), from, graph, sites);
            return walk(t, t.elem(v), from, graph, sites);
        }
        Ty::Fn(sig) => {
            let sig = t.sig(sig);
            for p in &sig.params {
                walk(t, p.ty, from, graph, sites);
            }
            return walk(t, sig.ret, from, graph, sites);
        }
        _ => return,
    };
    if args.is_empty() {
        return;
    }
    let site = sites.len();
    let targets = t.owner_params(to).to_vec();
    for (i, &a) in args.iter().enumerate() {
        let mut found = Vec::new();
        placeholders(t, a, &mut found);
        for p in found {
            graph.add(p, targets[i], a != Ty::Param(p), site);
        }
    }
    sites.push(Site { from, to, args: args.clone() });
    for a in args {
        walk(t, a, from, graph, sites);
    }
}

/// Every placeholder in `ty` (an instance's through its arguments; a declaration's own).
fn placeholders(t: &Tables, ty: Ty, found: &mut Vec<ParamId>) {
    match ty {
        Ty::Param(p) => {
            if !found.contains(&p) {
                found.push(p);
            }
        }
        Ty::Struct(id) => t.struct_parts(id).1.into_iter().for_each(|a| placeholders(t, a, found)),
        Ty::Enum(id) | Ty::Variant(id, _) => t.enum_parts(id).1.into_iter().for_each(|a| placeholders(t, a, found)),
        Ty::Array(e) | Ty::List(e) | Ty::Optional(e) => placeholders(t, t.elem(e), found),
        Ty::Map(k, v) => {
            placeholders(t, t.elem(k), found);
            placeholders(t, t.elem(v), found);
        }
        Ty::Fn(sig) => {
            let sig = t.sig(sig);
            for p in &sig.params {
                placeholders(t, p.ty, found);
            }
            placeholders(t, sig.ret, found);
        }
        _ => {}
    }
}

fn decl_item(t: &Tables, o: GenericOwner) -> usize {
    match o {
        GenericOwner::Struct(s) => t.strukt(s).item,
        GenericOwner::Enum(e) => t.enm(e).item,
        GenericOwner::Fn(f) => t.func(f).item,
    }
}

/// A tie-break for `decl_item` (never needed between items, which differ).
fn owner_key(o: GenericOwner) -> (u8, u32) {
    match o {
        GenericOwner::Struct(s) => (0, s.0),
        GenericOwner::Enum(e) => (1, e.0),
        GenericOwner::Fn(f) => (2, f.0),
    }
}

fn decl_span(t: &Tables, o: GenericOwner) -> kestrel_syntax::Span {
    match o {
        GenericOwner::Struct(s) => t.strukt(s).span,
        GenericOwner::Enum(e) => t.enm(e).span,
        GenericOwner::Fn(f) => t.func(f).span,
    }
}

fn decl_name(t: &Tables, o: GenericOwner) -> String {
    match o {
        GenericOwner::Struct(s) => t.strukt(s).name.clone(),
        GenericOwner::Enum(e) => t.enm(e).name.clone(),
        GenericOwner::Fn(f) => t.func(f).name.clone(),
    }
}

/// The error and its help for a cycle reported at `first`: the chain made by following a cycle
/// of written instances from `first` back to it twice (`Tree[T]` holds a `Tree[list[T]]`, which
/// holds a `Tree[list[list[T]]]`).
fn message(
    t: &Tables,
    first: GenericOwner,
    cycle: &[ParamId],
    owners: &HashSet<GenericOwner>,
    graph: &Graph,
    sites: &[Site],
) -> (String, String) {
    let name = decl_name(t, first);
    let own: Vec<String> = t.owner_params(first).iter().map(|&p| t.param_name(p).to_string()).collect();
    let written = |args: &[String]| format!("{name}[{}]", args.join(", "));
    let help = format!("use the same placeholders (`{}`), or types that don't grow", written(&own));
    let path = cycle_path(first, cycle, owners, graph, sites);
    let once = compose(t, &path, sites, own.clone());
    let twice = compose(t, &path, sites, once.clone());
    let msg = format!(
        "`{name}` would need endless copies: `{}` holds a `{}`, which holds a `{}`, and so on",
        written(&own),
        written(&once),
        written(&twice)
    );
    (msg, help)
}

/// Sites leading from `first` through a growing site of the cycle and back to `first`, each
/// between declarations of the cycle.
fn cycle_path(first: GenericOwner, cycle: &[ParamId], owners: &HashSet<GenericOwner>, graph: &Graph, sites: &[Site]) -> Vec<usize> {
    let grow = graph
        .edges
        .iter()
        .find(|e| e.growing && cycle.contains(&e.from) && cycle.contains(&e.to))
        .expect("a growing cycle has a growing edge")
        .site;
    let mut path = route(first, sites[grow].from, owners, sites);
    path.push(grow);
    path.extend(route(sites[grow].to, first, owners, sites));
    path
}

/// The shortest run of sites from `from` to `to` (empty when they are the same), between
/// declarations of `owners`.
fn route(from: GenericOwner, to: GenericOwner, owners: &HashSet<GenericOwner>, sites: &[Site]) -> Vec<usize> {
    let mut prev: HashMap<GenericOwner, usize> = HashMap::new();
    let mut queue = VecDeque::from([from]);
    let mut seen = HashSet::from([from]);
    while let Some(at) = queue.pop_front() {
        if at == to {
            break;
        }
        for (i, s) in sites.iter().enumerate() {
            if s.from == at && owners.contains(&s.to) && seen.insert(s.to) {
                prev.insert(s.to, i);
                queue.push_back(s.to);
            }
        }
    }
    let mut path = Vec::new();
    let mut at = to;
    while at != from {
        let Some(&i) = prev.get(&at) else { break };
        path.push(i);
        at = sites[i].from;
    }
    path.reverse();
    path
}

/// The arguments `first` is given at the end of `path`, written in terms of `start` (the text for
/// each of `first`'s placeholders).
fn compose(t: &Tables, path: &[usize], sites: &[Site], start: Vec<String>) -> Vec<String> {
    let mut env = start;
    for &i in path {
        let params = t.owner_params(sites[i].from).to_vec();
        env = sites[i].args.iter().map(|&a| show(t, a, &params, &env)).collect();
    }
    env
}

/// `ty` as `ty_name` writes it, with each of `params` written as its text in `env`.
fn show(t: &Tables, ty: Ty, params: &[ParamId], env: &[String]) -> String {
    let all = |tys: Vec<Ty>| tys.into_iter().map(|a| show(t, a, params, env)).collect::<Vec<_>>().join(", ");
    match ty {
        Ty::Param(p) => match params.iter().position(|&q| q == p) {
            Some(i) if i < env.len() => env[i].clone(),
            _ => t.param_name(p).to_string(),
        },
        Ty::Struct(id) => {
            let (def, args) = t.struct_parts(id);
            if args.is_empty() { t.ty_name(ty) } else { format!("{}[{}]", t.strukt(def).name, all(args)) }
        }
        Ty::Enum(id) => {
            let (def, args) = t.enum_parts(id);
            if args.is_empty() { t.ty_name(ty) } else { format!("{}[{}]", t.enm(def).name, all(args)) }
        }
        Ty::Array(e) => format!("[{}]", show(t, t.elem(e), params, env)),
        Ty::List(e) => format!("list[{}]", show(t, t.elem(e), params, env)),
        Ty::Optional(e) => format!("{}?", show(t, t.elem(e), params, env)),
        Ty::Map(k, v) => format!("map[{}, {}]", show(t, t.elem(k), params, env), show(t, t.elem(v), params, env)),
        Ty::Fn(sig) => {
            let sig = t.sig(sig);
            let ps: Vec<String> =
                sig.params.iter().map(|p| format!("{}{}", if p.is_var { "var " } else { "" }, show(t, p.ty, params, env))).collect();
            let ret = if sig.ret == Ty::Unit { String::new() } else { format!(" -> {}", show(t, sig.ret, params, env)) };
            format!("{}({}){ret}", if sig.is_pure { "pure " } else { "" }, ps.join(", "))
        }
        other => t.ty_name(other),
    }
}
