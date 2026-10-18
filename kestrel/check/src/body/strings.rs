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

    /// `s.name(...)`. The receiver is checked already. Every string a method gives is new, in the
    /// current arena (that is `region`'s default for a call).
    pub(crate) fn str_method(&mut self, e: &Expr, name: &str, name_span: Span, args: &[Arg]) -> Ty {
        // The method, how its arguments are described when they're wrong, and how many it takes.
        let (method, usage, count) = match name {
            "has" => (StrMethod::Has, "`has` takes one value", 1),
            "find" => (StrMethod::Find, "`find` takes one value", 1),
            "starts_with" => (StrMethod::StartsWith, "`starts_with` takes one value", 1),
            "ends_with" => (StrMethod::EndsWith, "`ends_with` takes one value", 1),
            "split" => (StrMethod::Split, "`split` takes one value", 1),
            "slice" => (StrMethod::Slice, "`slice` takes two values", 2),
            "upper" => (StrMethod::Upper, "`upper` takes no arguments", 0),
            "lower" => (StrMethod::Lower, "`lower` takes no arguments", 0),
            "trim" => (StrMethod::Trim, "`trim` takes no arguments", 0),
            "to_int" => (StrMethod::ToInt, "`to_int` takes no arguments", 0),
            "to_float" => (StrMethod::ToFloat, "`to_float` takes no arguments", 0),
            _ => {
                self.error_help(name_span, format!("string has no method `{name}`"), STR_METHODS);
                self.check_loose_args(args);
                return Ty::Error;
            }
        };
        if args.len() != count || args.iter().any(|a| a.label.is_some() || a.is_var) {
            self.error(e.span, usage);
            self.check_loose_args(args);
            return Ty::Error;
        }
        self.t.res.insert(e.id, Res::Str(method));
        match method {
            StrMethod::Slice => args.iter().for_each(|a| self.check_int_index(&a.value)),
            StrMethod::Has | StrMethod::Find | StrMethod::StartsWith | StrMethod::EndsWith | StrMethod::Split => {
                self.check_value(&args[0].value, Some(Ty::Str));
            }
            _ => {}
        }
        match method {
            StrMethod::Has | StrMethod::StartsWith | StrMethod::EndsWith => Ty::Bool,
            StrMethod::Find | StrMethod::ToInt => Ty::Optional(self.t.intern_elem(Ty::Int)),
            StrMethod::ToFloat => Ty::Optional(self.t.intern_elem(Ty::Float)),
            StrMethod::Split => Ty::List(self.t.intern_elem(Ty::Str)),
            StrMethod::Slice | StrMethod::Upper | StrMethod::Lower | StrMethod::Trim => Ty::Str,
            StrMethod::Len | StrMethod::At => unreachable!("not a method"),
        }
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
        // A constant or a field default is built again at each use, wherever that is; the new
        // string would be in whichever arena is current there. (A closure written in one runs when
        // it is called, in the arena current there, so its body is like any function's.)
        if self.fn_id.is_none() && !self.in_closure() {
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
