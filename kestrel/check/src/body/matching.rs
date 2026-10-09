use std::collections::HashMap;

use kestrel_syntax::ast::{Expr, ExprKind, Pattern};

use super::expr::is_value_ty;
use super::{BodyChecker, Expect};
use crate::ty::Ty;

impl<'a> BodyChecker<'a> {
    /// `match value { Variant { } Variant binding { } else { } }`: every
    /// variant handled once (or by `else`); typed like `if` with N branches.
    pub(crate) fn check_match(&mut self, e: &Expr, expect: Expect) -> Ty {
        let ExprKind::Match { scrutinee, arms } = &e.kind else { unreachable!("check_match on non-match") };
        let scrutinee_ty = self.check_expr(scrutinee, None);
        let enum_id = match scrutinee_ty {
            Ty::Enum(id) => Some(id),
            Ty::Error => None,
            other => {
                let msg = format!("`match` needs an enum value, found {}", self.tn(other));
                self.error(scrutinee.span, msg);
                None
            }
        };
        // Bindings alias the matched variable, so they can change it when it can change.
        let root = self.place_root(scrutinee);
        let path = self.path_key(scrutinee);
        let changeable = root.is_some_and(|r| self.immutable(r).is_none());
        let (enum_name, variants): (String, Vec<(String, usize)>) = match enum_id {
            Some(id) => {
                let info = self.t.enm(id);
                (info.name.clone(), info.variants.iter().map(|v| (v.name.clone(), v.fields.len())).collect())
            }
            None => (String::new(), Vec::new()),
        };
        let mut seen = vec![false; variants.len()];
        let mut has_else = false;
        let mut matched = Vec::with_capacity(arms.len());
        let mut tys = Vec::with_capacity(arms.len());
        let mut hint = match expect {
            Expect::Value(want) => want,
            Expect::Discard => None,
        };
        for arm in arms {
            if has_else {
                self.error(arm.span, "this arm can never run: the `else` above matches everything");
            }
            self.scopes.push(HashMap::new());
            let mut variant = None;
            let mut guarded = false;
            match &arm.pattern {
                Pattern::Else(_) => has_else = true,
                Pattern::Variant { name, name_span, binding } => match (enum_id, variants.iter().position(|(n, _)| n == name)) {
                    (None, _) => {}
                    (Some(_), None) => self.error(*name_span, format!("`{enum_name}` has no variant `{name}`")),
                    (Some(id), Some(v)) => {
                        if seen[v] {
                            self.error(*name_span, format!("`{name}` is matched twice"));
                        }
                        seen[v] = true;
                        variant = Some(v as u32);
                        if let Some(b) = binding {
                            if variants[v].1 == 0 {
                                self.error(b.span, format!("`{name}` has no fields to bind"));
                            }
                            let l = self.declare(b, Ty::Variant(id, v as u32), changeable, false, false);
                            self.t.locals[l.0 as usize].alias = root;
                            if let Some(root) = root {
                                self.match_guards.push(super::regions::MatchGuard { root, path: path.clone(), binding: l });
                                guarded = true;
                            }
                        }
                    }
                },
            }
            matched.push(variant);
            let arm_expect = match expect {
                Expect::Discard => Expect::Discard,
                Expect::Value(_) => Expect::Value(hint),
            };
            let ty = self.check_block(&arm.body, arm_expect);
            if matches!(expect, Expect::Value(_)) {
                if ty == Ty::Unit {
                    self.error(arm.body.span, "this branch has no value");
                }
                if hint.is_none() && is_value_ty(ty) {
                    hint = Some(ty);
                }
            }
            tys.push(ty);
            if guarded {
                self.match_guards.pop();
            }
            self.scopes.pop();
        }
        let missing: Vec<String> =
            variants.iter().zip(&seen).filter(|(_, done)| !**done).map(|((n, _), _)| format!("`{n}`")).collect();
        if enum_id.is_some() && !has_else && !missing.is_empty() {
            let msg = format!("`match` on `{enum_name}` doesn't handle {}", missing.join(", "));
            self.error_help(e.span, msg, "add arms for them, or an `else` arm");
        }
        self.t.match_arms.insert(e.id, matched);
        let all_never = !tys.is_empty() && tys.iter().all(|t| *t == Ty::Never);
        match expect {
            Expect::Discard if all_never => Ty::Never,
            Expect::Discard => Ty::Unit,
            Expect::Value(_) if tys.contains(&Ty::Unit) => Ty::Error,
            Expect::Value(_) if all_never => Ty::Never,
            Expect::Value(Some(want)) => want,
            Expect::Value(None) => tys.iter().copied().find(|t| *t != Ty::Never).unwrap_or(Ty::Error),
        }
    }
}
