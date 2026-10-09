use kestrel_syntax::ast::{Arg, Expr, ExprKind, Item, Program};
use kestrel_syntax::Diagnostic;

use super::place::Immutable;
use super::{is_const_expr, BodyChecker};
use crate::tables::{LocalId, Res, StructId, Tables};
use crate::ty::Ty;

/// Field defaults are constant expressions of the field's type.
pub(crate) fn check_field_defaults(program: &Program, t: &mut Tables, diags: &mut Vec<Diagnostic>) {
    let structs = program.items.iter().filter_map(|it| match it {
        Item::Struct(s) => Some(s),
        _ => None,
    });
    for (sid, s) in structs.enumerate() {
        for f in &s.fields {
            let Some(default) = &f.default else { continue };
            let Some(field) = t.structs[sid].fields.iter().find(|g| g.name == f.name) else { continue };
            let ty = field.ty;
            let mut bc = BodyChecker::new(t, diags, None, Ty::Unit, false);
            if is_const_expr(default) {
                bc.check_value(default, Some(ty));
            } else {
                bc.error(default.span, "field defaults can only use literals, operators and constants");
            }
        }
    }
}

impl<'a> BodyChecker<'a> {
    /// `Point(1, y: 2)`: arguments bind to fields like parameters; defaults fill the rest.
    pub(crate) fn check_construct(&mut self, e: &Expr, sid: StructId, args: &[Arg]) -> Ty {
        self.t.res.insert(e.id, Res::Construct(sid));
        let info = self.t.strukt(sid).clone();
        let names: Vec<&str> = info.fields.iter().map(|f| f.name.as_str()).collect();
        let (slots, bound) = self.bind_args(&info.name, &names, args, "field");
        let mut ok = bound;
        for (fi, slot) in slots.iter().enumerate() {
            match *slot {
                Some(ai) => {
                    let a = &args[ai];
                    if a.is_var {
                        self.error(a.span, "constructor arguments can't be `var`");
                    }
                    self.check_value(&a.value, Some(info.fields[fi].ty));
                }
                None if bound && !info.fields[fi].has_default => {
                    self.error(e.span, format!("missing field `{}` for `{}`", info.fields[fi].name, info.name));
                    ok = false;
                }
                None => {}
            }
        }
        for (ai, a) in args.iter().enumerate() {
            if !slots.contains(&Some(ai)) {
                self.check_expr(&a.value, None);
            }
        }
        if ok {
            self.t.ctor_args.insert(e.id, slots);
        }
        Ty::Struct(sid)
    }

    pub(crate) fn check_field(&mut self, e: &Expr) -> Ty {
        let ExprKind::Field { base, name, name_span } = &e.kind else { unreachable!("check_field on non-field") };
        if let Some(id) = self.enum_named(base) {
            return self.variant_value(e, id, name, *name_span);
        }
        let base_ty = self.check_field_base(base);
        let (owner, fields) = match base_ty {
            Ty::Struct(sid) => {
                let info = self.t.strukt(sid);
                (info.name.clone(), &info.fields)
            }
            Ty::Variant(id, v) => (self.tn(base_ty), &self.t.enm(id).variants[v as usize].fields),
            Ty::Error => return Ty::Error,
            other => {
                let msg = format!("{} has no field `{name}`", self.tn(other));
                self.error(*name_span, msg);
                return Ty::Error;
            }
        };
        match fields.iter().position(|f| &f.name == name) {
            Some(i) => {
                let ty = fields[i].ty;
                self.t.field_index.insert(e.id, i as u32);
                ty
            }
            None => {
                self.error(*name_span, format!("`{owner}` has no field `{name}`"));
                Ty::Error
            }
        }
    }

    pub(crate) fn check_method_call(&mut self, e: &Expr) -> Ty {
        let ExprKind::MethodCall { receiver, name, name_span, args } = &e.kind else { unreachable!("not a method call") };
        if let Some(id) = self.enum_named(receiver) {
            return self.variant_construct(e, id, name, *name_span, args);
        }
        let receiver_ty = self.check_expr(receiver, None);
        let found = match receiver_ty {
            Ty::Struct(sid) => {
                let info = self.t.strukt(sid);
                info.methods.get(name).copied().ok_or_else(|| Some(format!("`{}` has no method `{name}`", info.name)))
            }
            Ty::Enum(id) => {
                let info = self.t.enm(id);
                info.methods.get(name).copied().ok_or_else(|| Some(format!("`{}` has no method `{name}`", info.name)))
            }
            Ty::Error => Err(None),
            other => Err(Some(format!("{} has no method `{name}`", self.tn(other)))),
        };
        let fid = match found {
            Ok(fid) => fid,
            Err(msg) => {
                if let Some(msg) = msg {
                    self.error(*name_span, msg);
                }
                for a in args {
                    self.check_expr(&a.value, None);
                }
                return Ty::Error;
            }
        };
        self.t.res.insert(e.id, Res::Fn(fid));
        self.check_callee_purity(*name_span, fid);
        let mut var_roots = Vec::new();
        if self.t.func(fid).params[0].is_var {
            if let Some(root) = self.receiver_root(receiver, name) {
                var_roots.push((root, None));
            }
        }
        self.check_args(e, fid, args, 1, var_roots)
    }

    /// The variable a `var self` call changes, once it's known it may change.
    fn receiver_root(&mut self, receiver: &Expr, method: &str) -> Option<LocalId> {
        let Some(root) = self.place_root(receiver) else {
            let msg = format!("`{method}` changes its receiver, so it needs a variable, not a temporary");
            self.error(receiver.span, msg);
            return None;
        };
        let name = self.t.local(root).name.clone();
        let msg = match self.immutable(root) {
            None => {
                self.mark_changed(root);
                return Some(root);
            }
            Some(Immutable::Let) => format!("can't call `{method}` on `{name}`: it changes `{name}`, which was declared with `let`"),
            Some(Immutable::Param) => format!("can't call `{method}` on parameter `{name}`: it changes `{name}`, which isn't `var`"),
            Some(Immutable::SelfParam) => {
                format!("can't call `{method}` on `self`: it changes `self`, but the method takes `self`, not `var self`")
            }
        };
        self.error(receiver.span, msg);
        None
    }
}
