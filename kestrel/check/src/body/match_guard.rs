//! The `match` arm guard: while an arm's binding reads from a place, nothing else may change it.

use kestrel_syntax::ast::Expr;
use kestrel_syntax::Span;

use super::regions::PathKey;
use super::BodyChecker;
use crate::tables::LocalId;

impl<'a> BodyChecker<'a> {
    /// While a `match` arm's binding reads from a place, nothing may change
    /// that place (or anything holding it) except through the binding:
    /// the binding would then read a different variant's fields.
    pub(crate) fn check_not_matched(&mut self, target: &Expr) {
        let Some(root) = self.place_root(target) else { return };
        let Some(path) = self.path_key(target) else { return };
        if let Some(binding) = self.matched_by(root, &path) {
            let name = self.t.local(root).name.clone();
            self.matched_error(target.span, &name, binding);
        }
    }

    /// The binding of a `match` arm that is reading from the place `path` (whose variable, aliases
    /// followed, is `root`), or from a place that holds it.
    pub(crate) fn matched_by(&self, root: LocalId, path: &PathKey) -> Option<LocalId> {
        let g = self.match_guards.iter().find(|g| {
            path.root != g.binding
                && root == g.root
                && match &g.path {
                    Some(s) if s.root == path.root => {
                        path.steps.len() <= s.steps.len()
                            && path.steps.iter().zip(&s.steps).all(|(a, b)| a.is_none() || b.is_none() || a == b)
                    }
                    _ => true,
                }
        })?;
        Some(g.binding)
    }

    /// Changing `name` at `span` would change what `binding`, a `match` arm's binding, reads.
    pub(crate) fn matched_error(&mut self, span: Span, name: &str, binding: LocalId) {
        let binding = self.t.local(binding).name.clone();
        self.error_help(
            span,
            format!("can't change `{name}` here: this `match` arm's `{binding}` reads from it"),
            format!("change it after the `match`, or copy what you need out of `{binding}` first"),
        );
    }
}
