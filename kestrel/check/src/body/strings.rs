//! Strings: `.len`, `s[i]`, `for c in s` and `a + b`. Strings are UTF-8 and count in characters.
//! Every string these give is new, in the current arena, like an interpolation; a string is never
//! a place, so it can't be changed in place (see `regions` for where the results may be stored).

use std::collections::HashMap;

use kestrel_syntax::ast::{Arg, Binding, Block, Expr};
use kestrel_syntax::Span;

use super::remove::Over;
use super::BodyChecker;
use crate::tables::{Res, StrMethod};
use crate::ty::Ty;

/// The help for a method a string doesn't have.
const STR_METHODS: &str =
    "strings have `has`, `find`, `starts_with`, `ends_with`, `slice`, `split`, `upper`, `lower`, `trim`, `to_int`, `to_float` and `.len`";

/// The error for `s[i] = x` and `s[i] += x`.
pub(super) const STR_IN_PLACE: &str = "strings can't be changed in place; build a new one with `+` or `\"{...}\"`";

impl<'a> BodyChecker<'a> {
    /// `s.len`, the only field a string has.
    pub(crate) fn str_field(&mut self, e: &Expr, name: &str, span: Span) -> Ty {
        if name == "len" {
            self.t.res.insert(e.id, Res::Str(StrMethod::Len));
            return Ty::Int;
        }
        self.error_help(span, format!("string has no field `{name}`"), "its length is `.len`");
        Ty::Error
    }

    /// `s.name(...)`. The receiver is checked already.
    pub(crate) fn str_method(&mut self, name: &str, name_span: Span, args: &[Arg]) -> Ty {
        self.error_help(name_span, format!("string has no method `{name}`"), STR_METHODS);
        self.check_loose_args(args);
        Ty::Error
    }

    /// `s[i]`: the character at `i`, as a new one-character string.
    pub(crate) fn check_str_index(&mut self, e: &Expr, index: &Expr) -> Ty {
        self.check_int_index(index);
        self.t.res.insert(e.id, Res::Str(StrMethod::At));
        Ty::Str
    }

    /// `a + b`: two strings make a new one. The checker has both sides' types.
    pub(crate) fn str_concat(&mut self, e: &Expr, lt: Ty, rt: Ty) -> Ty {
        if lt == Ty::Error || rt == Ty::Error {
            return Ty::Error;
        }
        if lt != Ty::Str || rt != Ty::Str {
            let msg = format!("`+` needs two numbers or two strings, found {} and {}", self.tn(lt), self.tn(rt));
            self.error_help(e.span, msg, "put a number in a string with `\"{n}\"`");
            return Ty::Error;
        }
        // A constant or a field default is built again at each use, wherever that is; the
        // new string would be in whichever arena is current there.
        if self.fn_id.is_none() {
            self.error(e.span, "a constant or field default can't join strings with `+`; write the text as one string");
            return Ty::Error;
        }
        Ty::Str
    }

    /// `for c in s` / `for i, c in s`: each character as a new string (`i` counts characters). The
    /// loop walks the string as it was when it started. `c` is a plain local, not a view: every
    /// pass gives it a new string.
    pub(crate) fn check_str_for_in(&mut self, par: bool, index: Option<&Binding>, var: &Binding, iter: &Expr, body: &Block) {
        if par {
            self.error(iter.span, "`par for` loops over an array or a list");
        }
        self.scopes.push(HashMap::new());
        if let Some(i) = index {
            self.declare(i, Ty::Int, false, false, false);
        }
        let c = self.declare(var, Ty::Str, false, false, false);
        self.enter_item_loop(c, Over::Str, iter, body);
        let entry = self.moved.clone();
        self.item_loop_body(Some(c), entry.clone(), Some(entry), body);
        self.leave_item_loop();
        self.scopes.pop();
    }
}
