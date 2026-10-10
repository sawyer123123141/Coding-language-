//! `par for` bodies. Iterations run at the same time, so a body may change
//! only the loop's item and its own variables, call only `pure` functions,
//! and can't print, `return` or `break`. It may read anything except the
//! collection's other items while it changes items.

use std::collections::HashSet;

use kestrel_syntax::ast::Expr;
use kestrel_syntax::Span;

use super::regions::PathKey;
use super::BodyChecker;
use crate::tables::LocalId;

pub(crate) struct ParBody {
    /// The loop's item: a view into the collection.
    pub(crate) item: LocalId,
    /// The collection, when it's a place.
    collection: Option<PathKey>,
    /// Locals from this id on are the body's own (one per iteration).
    first_local: u32,
    /// Views into the item: inner loop elements and `match` bindings.
    item_views: HashSet<LocalId>,
    /// The loop depth (`loop_breaks.len()`) of the `par for` itself.
    pub(crate) depth: usize,
    changes_item: bool,
    /// Places read in the body that share the collection's root.
    collection_reads: Vec<(Span, PathKey)>,
}

impl ParBody {
    pub(crate) fn new(item: LocalId, collection: Option<PathKey>, first_local: u32, depth: usize) -> Self {
        ParBody { item, collection, first_local, item_views: HashSet::new(), depth, changes_item: false, collection_reads: Vec::new() }
    }
}

impl<'a> BodyChecker<'a> {
    /// Whether a place rooted at `root` (aliases not followed) belongs to
    /// one iteration of `par`: the item, a view into it, or a variable the
    /// body declared that isn't a view of something else.
    fn owned_by(&self, par: &ParBody, root: LocalId) -> bool {
        root == par.item || par.item_views.contains(&root) || (root.0 >= par.first_local && self.t.local(root).alias.is_none())
    }

    /// Before a write to the place `e` (assignment, `var` argument or
    /// receiver, `in`): inside `par for` bodies it must belong to one
    /// iteration of every one of them.
    pub(crate) fn check_par_write(&mut self, e: &Expr) {
        if self.par.is_empty() {
            return;
        }
        let Some(key) = self.path_key(e) else { return };
        if self.par.iter().all(|p| self.owned_by(p, key.root)) {
            for p in &mut self.par {
                if key.root == p.item || p.item_views.contains(&key.root) {
                    p.changes_item = true;
                }
            }
            return;
        }
        let item = self.t.local(self.par.last().expect("checked above").item).name.clone();
        let msg = format!(
            "`par for` iterations run at the same time, so this can't change `{}`; only the loop's item (`{item}`) and the body's own variables can change",
            self.path_name(&key)
        );
        self.error(e.span, msg);
    }

    /// A view (a loop element or `match` binding) declared over `of`: it
    /// belongs to an iteration when `of` does.
    pub(crate) fn note_view(&mut self, view: LocalId, of: &Expr) {
        let Some(key) = self.path_key(of) else { return };
        for i in 0..self.par.len() {
            if self.owned_by(&self.par[i], key.root) {
                self.par[i].item_views.insert(view);
            }
        }
    }

    /// A place `e` (a variable, field or element) the body uses.
    pub(crate) fn note_par_read(&mut self, e: &Expr) {
        if self.par.is_empty() {
            return;
        }
        let Some(key) = self.path_key(e) else { return };
        for p in &mut self.par {
            if p.collection.as_ref().is_some_and(|c| c.root == key.root) {
                p.collection_reads.push((e.span, key.clone()));
            }
        }
    }

    /// Ends a `par for` body: reading the collection while changing its
    /// items would race with the other iterations. Only whole places count
    /// (`w.score` reads nothing of `w.chunks`, though `w` is inside it).
    pub(crate) fn end_par(&mut self) {
        let par = self.par.pop().expect("a `par for` body is open");
        let (Some(collection), true) = (&par.collection, par.changes_item) else { return };
        let (name, item) = (self.path_name(collection), self.t.local(par.item).name.clone());
        let reads = &par.collection_reads;
        for (span, key) in reads {
            let inner = reads.iter().any(|(s, _)| s != span && s.start <= span.start && span.end <= s.end);
            if !inner && key.overlaps(collection) {
                let msg = format!("this reads `{name}`, whose items other iterations are changing; use the loop's item (`{item}`)");
                self.error(*span, msg);
            }
        }
    }

    /// Inside a `par for` body: the error for calling `name` unless it's pure.
    pub(crate) fn par_call_error(&self, name: &str) -> Option<(String, String)> {
        (!self.par.is_empty()).then(|| {
            (
                format!("`par for` bodies can only call `pure` functions; `{name}` isn't `pure`"),
                format!("mark `{name}` as `pure` if it changes nothing but its `var` arguments"),
            )
        })
    }
}
