//! What a `T` can do in generic code (R1), part one: methods and fields. A method call on a `T`
//! looks the name up in `T`'s rules (interfaces) and is then checked as a call through an interface
//! value is, against the interface's abstract method. A field of a `T` is always an error: nothing
//! is known about `T` beyond its rules.

use kestrel_syntax::ast::Expr;
use kestrel_syntax::Span;

use super::lists::written;
use super::BodyChecker;
use crate::generic::ParamId;
use crate::tables::{FnId, IfaceId, Res};
use crate::ty::Ty;

impl<'a> BodyChecker<'a> {
    /// The method `name` of placeholder `p`: the abstract method of one of its rules and the
    /// `Res::ParamCall` that names it, or the message for why there is none.
    ///
    /// When several rules have a method of that name (G9 made them the same signature, but they
    /// may differ in `pure`), the first whose method is `pure` is taken, else the first that has
    /// it: a type that meets every rule has a pure method when any rule's is, so `[T: A + B]` and
    /// `[T: B + A]` give the same answer (P26).
    pub(super) fn param_method(&self, p: ParamId, name: &str) -> Result<(FnId, Res), String> {
        let rules: Vec<(IfaceId, FnId, u32)> = self
            .t
            .generic
            .param(p)
            .ifaces
            .iter()
            .filter_map(|&i| self.iface_method(i, name).map(|(f, slot)| (i, f, slot)))
            .collect();
        let pick = rules.iter().find(|&&(_, f, _)| self.t.func(f).is_pure).or(rules.first());
        if let Some(&(iface, f, slot)) = pick {
            return Ok((f, Res::ParamCall { param: p, iface, slot }));
        }
        let pn = self.t.param_name(p);
        let known = self.t.rule_names(p);
        if known.is_empty() {
            return Err(format!("`{pn}` has no rules, so it has no methods; add one, like `[{pn}: Damageable]`"));
        }
        let known: Vec<String> = known.iter().map(|r| format!("`{r}`")).collect();
        Err(format!("`{pn}` has no method `{name}`: its rules ({}) don't have one", known.join(", ")))
    }

    /// `x.hp` where `x` is a `T`.
    pub(super) fn param_field_error(&mut self, base: &Expr, p: ParamId, field: &str, span: Span) -> Ty {
        let pn = self.t.param_name(p).to_string();
        let mut chars = field.chars();
        let capital: String = chars.next().map(|c| c.to_ascii_uppercase()).into_iter().chain(chars).collect();
        let msg = format!("`{}` is a `{pn}`, which can be any type, so it has no field `{field}`", written(base));
        self.error_help(span, msg, format!("give `{pn}` a rule with a method instead, like `[{pn}: Has{capital}]`"));
        Ty::Error
    }
}
