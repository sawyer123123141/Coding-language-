//! The help for a wider int where a narrower one goes (an `i64` where an `int` does): the
//! conversion stops the program in a normal build when the value doesn't fit (a checksum's `i64`
//! rarely does), so keeping the wider type comes first, named after what the value goes into.

use kestrel_syntax::ast::{Expr, ExprId};

use super::BodyChecker;
use crate::ty::Ty;

/// What a value checked against a type goes into.
#[derive(Clone, Copy)]
pub(crate) enum Site {
    Variable,
    Parameter,
    Field,
    Return,
    /// An element of a list or array (`ys[0] = a`).
    Item,
    /// A map's value (`m[k] = a`).
    MapValue,
}

impl<'a> BodyChecker<'a> {
    /// Runs `f` with `e` known to go into `into`, for the narrowing help (see `narrowing_help`).
    pub(crate) fn going_into<T>(&mut self, e: &Expr, into: Site, f: impl FnOnce(&mut Self) -> T) -> T {
        let saved = self.into_site.replace((e.id, into));
        let result = f(self);
        self.into_site = saved;
        result
    }

    /// Marks the expression `id` as going into `into` until it is cleared; for a function's
    /// result, whose check runs through the whole body.
    pub(crate) fn set_into(&mut self, site: Option<(ExprId, Site)>) {
        self.into_site = site;
    }

    /// The help for `e`, a `from` where the narrower int `to` goes.
    pub(crate) fn narrowing_help(&self, e: &Expr, from: Ty, to: Ty) -> String {
        let (f, t) = (from.name(), to.name());
        let noun = match self.into_site {
            Some((id, into)) if id == e.id => match into {
                Site::Variable => "variable's type",
                Site::Parameter => "parameter's type",
                Site::Field => "field's type",
                Site::Return => "return type",
                Site::Item => "list's item type",
                Site::MapValue => "map's value type",
            },
            _ => "type of what it goes into",
        };
        format!(
            "keep it an `{f}`: make the {noun} `{f}`; `{t}(...)` also works, but in a normal build it stops the program if the value doesn't fit in `{t}` (a `--release` build wraps)"
        )
    }
}
