//! Method calls on an instance of a generic type (`s.push(3)` on a `Stack[int]`), or on the
//! declaration itself (`self.size()` inside `Stack`'s own methods): the method is checked against
//! its signature with the receiver's arguments put in for the type's placeholders, and the
//! arguments are recorded as the call's `type_args` for lowering.

use kestrel_syntax::ast::Expr;

use super::args::CallSig;
use super::BodyChecker;
use crate::generic::{GenericOwner, Subst};
use crate::tables::{FnId, ParamSig};
use crate::ty::Ty;

impl<'a> BodyChecker<'a> {
    /// The signature the method call `e` of `fid` on a receiver of type `receiver_ty` is checked
    /// against. A method of a plain type (or an interface's) is checked as declared.
    pub(super) fn method_sig(&mut self, e: &Expr, fid: FnId, receiver_ty: Ty) -> CallSig {
        let plain = CallSig::of(self.t, fid);
        let Some(owner @ (GenericOwner::Struct(_) | GenericOwner::Enum(_))) = self.t.func(fid).generic else { return plain };
        // The receiver is an instance (its own arguments) or the declaration itself, whose
        // arguments are its own placeholders.
        let args = match (receiver_ty, owner) {
            (Ty::Struct(s), GenericOwner::Struct(def)) => {
                let (found, args) = self.t.struct_parts(s);
                assert_eq!(found, def, "a method of one generic type called on another; this is a compiler bug");
                args
            }
            (Ty::Enum(id) | Ty::Variant(id, _), GenericOwner::Enum(def)) => {
                let (found, args) = self.t.enum_parts(id);
                assert_eq!(found, def, "a method of one generic type called on another; this is a compiler bug");
                args
            }
            _ => return plain,
        };
        let args = self.t.intern_ty_list(args);
        self.t.generic.type_args.insert(e.id, args);
        self.subst_call_sig(plain, Subst { owner, args })
    }

    /// `plain`'s parameters and result with `s`'s arguments put in for its owner's placeholders.
    pub(super) fn subst_call_sig(&mut self, plain: CallSig, s: Subst) -> CallSig {
        let params = plain.params.iter().map(|p| ParamSig { name: p.name.clone(), ty: self.t.subst(p.ty, s), is_var: p.is_var }).collect();
        let ret = self.t.subst(plain.ret, s);
        CallSig { fid: plain.fid, params, ret }
    }
}
