//! The vector makes that aren't one number per component (`Tables::vec_makes`): a fill
//! (`vec3(0.5)`), a copy or conversion (`vec3(v)`, `vec3(iv)`) and a mixed make (`vec3(v2, 1)`).

use kestrel_check::StructId;
use kestrel_syntax::ast;

use super::builtins::node;
use super::FnLowerer;
use crate::ir::*;

impl<'a> FnLowerer<'a> {
    /// Each part is evaluated once, in source order (anything but a literal into a temporary),
    /// then its components are read: a number is one component, or every one when it is alone,
    /// and a vector gives its components, converted when its kind is the other one (as `int(x)`
    /// and `float(n)` convert).
    pub(crate) fn vec_make(&mut self, sid: StructId, args: &[ast::Arg], ty: KTy, line: u32) -> ExprKind {
        let s = StructIdx(sid.0);
        let (dims, lane) = (self.dims(s), self.lane(s));
        let mut pre = Vec::new();
        let mut fields = Vec::with_capacity(dims as usize);
        for a in args {
            let v = self.expr(&a.value);
            let v = self.once(v, &mut pre);
            let KTy::Struct(from) = v.ty else {
                fields.push(v);
                continue;
            };
            let from_lane = self.lane(from);
            let o = self.operand(v, &mut pre);
            for i in 0..self.dims(from) {
                let c = Self::component(&o, i, from_lane, line);
                fields.push(if from_lane == lane { c } else { node(ExprKind::Cast(Box::new(c)), lane, line) });
            }
        }
        if let [one] = fields.as_slice() {
            fields = vec![one.clone(); dims as usize];
        }
        Self::with_pre(pre, ExprKind::StructLit(s, fields), ty, line)
    }
}
