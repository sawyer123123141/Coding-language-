use kestrel_syntax::ast::{Arg, Expr, ExprKind, Item, Program};
use kestrel_syntax::Diagnostic;

use super::place::Immutable;
use super::{is_const_expr, BodyChecker};
use crate::builtins::Builtin;
use crate::tables::{LocalId, Res, StructId, Tables, BUILTIN_STRUCTS, VEC2, VEC3};
use crate::ty::Ty;

/// Field defaults are constant expressions of the field's type (see `is_const_expr`), built again
/// at every construction that leaves the field out. They can't build a value that owns an arena or
/// a GPU object (other than a fresh `Arena()`).
pub(crate) fn check_field_defaults(program: &Program, t: &mut Tables, diags: &mut Vec<Diagnostic>) {
    let structs = program.items.iter().filter_map(|it| match it {
        Item::Struct(s) => Some(s),
        _ => None,
    });
    for (sid, s) in structs.enumerate() {
        for f in &s.fields {
            let Some(default) = &f.default else { continue };
            let sid = sid + BUILTIN_STRUCTS as usize;
            let Some(field) = t.structs[sid].fields.iter().find(|g| g.name == f.name) else { continue };
            let ty = field.ty;
            let mut bc = BodyChecker::new(t, diags, None, Ty::Unit, false);
            if !is_const_expr(default, bc.t) {
                bc.error(default.span, "field defaults can only use literals, operators, constants, struct and enum values, and pure built-ins like `sqrt`");
            } else if bc.check_value(default, Some(ty)) != Ty::Error && builds_owning(default, bc.t) {
                bc.error(default.span, "field defaults can't build a value that owns an `Arena` or a GPU object; set the field when you build it");
            }
        }
    }
}

/// Whether the checked default `e` builds a value that owns an `Arena` or a GPU object, other
/// than a fresh `Arena()`.
fn builds_owning(e: &Expr, t: &Tables) -> bool {
    let any = |args: &[Arg]| args.iter().any(|a| builds_owning(&a.value, t));
    match &e.kind {
        ExprKind::Call { name, .. } if name == "Arena" => false,
        ExprKind::Call { args, .. } | ExprKind::MethodCall { args, .. } => t.is_owning(t.ty(e.id)) || any(args),
        // A constant is built again at each use, so naming an owning one builds one. One that is
        // just an `Arena` is a fresh `Arena()` each time, like writing it out.
        ExprKind::Ident(_) => matches!(t.res.get(&e.id), Some(Res::Const(_))) && t.ty(e.id) != Ty::Arena && t.is_owning(t.ty(e.id)),
        ExprKind::TypeCall { args, .. } => any(args),
        ExprKind::Unary { expr, .. } => builds_owning(expr, t),
        ExprKind::Binary { lhs, rhs, .. } => builds_owning(lhs, t) || builds_owning(rhs, t),
        ExprKind::ArrayLit(items) => items.iter().any(|i| builds_owning(i, t)),
        ExprKind::MapLit(entries) => entries.iter().any(|(k, v)| builds_owning(k, t) || builds_owning(v, t)),
        _ => false,
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
        // In source order, the order they run in: a move must come after every read.
        for (ai, a) in args.iter().enumerate() {
            let Some(fi) = slots.iter().position(|s| *s == Some(ai)) else {
                self.check_expr(&a.value, None);
                continue;
            };
            if a.is_var {
                self.error(a.span, "constructor arguments can't be `var`");
            }
            self.check_value(&a.value, Some(info.fields[fi].ty));
            self.consume(&a.value);
        }
        // `vec2()` and `vec3()` are zero vectors, like Godot's `Vector2()`.
        let vector = matches!(sid, VEC2 | VEC3);
        let zero_vector = vector && args.is_empty();
        let missing: Vec<String> = slots
            .iter()
            .enumerate()
            .filter(|&(fi, slot)| slot.is_none() && !info.fields[fi].has_default && !zero_vector)
            .map(|(fi, _)| format!("`{}`", info.fields[fi].name))
            .collect();
        if bound && !missing.is_empty() {
            let msg = match missing.as_slice() {
                [one] => format!("missing field {one} for `{}`", info.name),
                [init @ .., last] => format!("missing fields {} and {last} for `{}`", init.join(", "), info.name),
                [] => unreachable!("not empty"),
            };
            if vector {
                self.error_help(e.span, msg, format!("give every component, or none for a zero vector: `{}()`", info.name));
            } else {
                self.error(e.span, msg);
            }
            ok = false;
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
        if matches!(base_ty, Ty::Array(_) | Ty::List(_) | Ty::Map(..)) {
            return self.collection_field(base_ty, name, *name_span);
        }
        if base_ty == Ty::Str {
            return self.str_field(e, name, *name_span);
        }
        if self.method_value_error(base_ty, name, *name_span) {
            return Ty::Error;
        }
        let (owner, fields) = match base_ty {
            Ty::Struct(sid) => {
                let info = self.t.strukt(sid);
                (info.name.clone(), &info.fields)
            }
            Ty::Variant(id, v) => (self.tn(base_ty), &self.t.enm(id).variants[v as usize].fields),
            Ty::Error => return Ty::Error,
            Ty::Optional(_) => {
                let msg = format!("{} may be `none`, so it has no field `{name}`", self.tn(base_ty));
                self.error_help(*name_span, msg, "unwrap it first: `let x = ... else { ... }`");
                return Ty::Error;
            }
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
            None => match self.swizzle(base_ty, name) {
                Some((comps, ty)) => {
                    self.t.swizzles.insert(e.id, comps);
                    ty
                }
                None => {
                    self.error(*name_span, format!("`{owner}` has no field `{name}`"));
                    Ty::Error
                }
            },
        }
    }

    pub(crate) fn check_method_call(&mut self, e: &Expr) -> Ty {
        let ExprKind::MethodCall { receiver, name, name_span, args } = &e.kind else { unreachable!("not a method call") };
        if let Some(id) = self.enum_named(receiver) {
            return self.variant_construct(e, id, name, *name_span, args);
        }
        let receiver_ty = self.check_expr(receiver, None);
        if matches!(receiver_ty, Ty::Array(_) | Ty::List(_)) {
            return self.list_method(e, receiver, receiver_ty, args);
        }
        if let Ty::Map(..) = receiver_ty {
            return self.map_method(e, receiver, receiver_ty, args);
        }
        if receiver_ty == Ty::Str {
            return self.str_method(e, name, *name_span, args);
        }
        if let (Some(_), Some(b)) = (self.t.vec_dims(receiver_ty), Builtin::vector_method(name)) {
            return self.check_vector_method(e, b, receiver, receiver_ty, args);
        }
        // `b.on_click()` calls a field of function type, when no method has that name.
        if let Some((field, sig)) = self.fn_field(receiver_ty, name) {
            return self.check_field_call(e, receiver, name, field, sig, args);
        }
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
        let by_pointer = self.t.func(fid).params[0].is_var || receiver_ty.is_aggregate();
        let mut read_root = None;
        if self.t.func(fid).params[0].is_var {
            if let Some(root) = self.receiver_root(receiver, name) {
                var_roots.push((root, None));
            }
        } else if receiver_ty.is_aggregate() {
            read_root = self.place_root(receiver);
        }
        let ret = self.check_args(e, fid, args, 1, var_roots, read_root);
        if by_pointer {
            let later: Vec<&Expr> = args.iter().map(|a| &a.value).collect();
            self.check_changed_later(&[receiver], &later);
        }
        ret
    }

    /// `v.xz`, `v.zyx`: 2 or 3 components of a vector, as a new vector.
    fn swizzle(&self, base_ty: Ty, name: &str) -> Option<(Vec<u32>, Ty)> {
        let dims = self.t.vec_dims(base_ty)?;
        let comps: Option<Vec<u32>> = name.chars().map(|c| "xyz"[..dims as usize].find(c).map(|i| i as u32)).collect();
        let comps = comps?;
        let ty = match comps.len() {
            2 => Ty::Struct(crate::tables::VEC2),
            3 => Ty::Struct(crate::tables::VEC3),
            _ => return None,
        };
        Some((comps, ty))
    }

    /// The variable a `var self` call changes, once it's known it may change.
    pub(super) fn receiver_root(&mut self, receiver: &Expr, method: &str) -> Option<LocalId> {
        let Some(root) = self.place_root(receiver) else {
            let msg = format!("`{method}` changes its receiver, so it needs a variable, not a temporary");
            self.error(receiver.span, msg);
            return None;
        };
        self.check_not_matched(receiver);
        let name = self.t.local(root).name.clone();
        let msg = match self.immutable(root) {
            None => {
                self.mark_changed(root);
                self.check_par_write(receiver);
                return Some(root);
            }
            Some(Immutable::Let) => format!("can't call `{method}` on `{name}`: it changes `{name}`, which was declared with `let`"),
            Some(Immutable::Param) => format!("can't call `{method}` on parameter `{name}`: it changes `{name}`, which isn't `var`"),
            Some(Immutable::SelfParam) => {
                format!("can't call `{method}` on `self`: it changes `self`, but the method takes `self`, not `var self`")
            }
            Some(Immutable::Captured) => Self::captured_message(&name),
        };
        self.error(receiver.span, msg);
        None
    }
}
