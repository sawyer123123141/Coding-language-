//! Built-ins on vectors (one rule for every vector type, pick 4): `min max clamp abs` on every
//! vector and `floor ceil round sqrt lerp` on float vectors, lane by lane, with a plain number
//! filling every lane in any argument (V10); `v.sum()` on every vector; the float vectors'
//! methods (V11) and `cross` on a `vec3` (V12).

use kestrel_syntax::ast::{Arg, Expr};

use super::builtins::{args_problem, is_float_function};
use super::BodyChecker;
use crate::builtins::Builtin;
use crate::tables::Res;
use crate::ty::Ty;

impl<'a> BodyChecker<'a> {
    /// `min max clamp abs floor ceil round sqrt`: numbers as before, or vectors of one type and
    /// numbers. The values are checked in order; a literal takes the type of the numbers before it
    /// (`min`... only, as `+` does), or the lane type once a vector has come.
    pub(super) fn check_lane_call(&mut self, e: &Expr, b: Builtin, args: &[Arg]) -> Ty {
        let met_hint = !is_float_function(b);
        let mut tys = Vec::with_capacity(args.len());
        let (mut met, mut lane): (Option<Ty>, Option<Ty>) = (None, None);
        for a in args {
            let hint = lane.or(if met_hint { met } else { None });
            let ty = self.check_expr(&a.value, hint);
            let ty = self.reject_never(&a.value, ty);
            if let Some(k) = self.t.vector(ty) {
                lane.get_or_insert(k.lane());
                tys.push(ty);
                continue;
            }
            let ty = self.require_number(b, &a.value, ty);
            met = Some(met.and_then(|m| Ty::common(m, ty)).unwrap_or(ty)).filter(|t| t.is_numeric());
            tys.push(ty);
        }
        if lane.is_some() {
            return self.check_lane_values(e, b, args, &tys);
        }
        if met_hint {
            if tys.contains(&Ty::Error) {
                return Ty::Error;
            }
            return self.meet_args(e, b, args, &tys);
        }
        // The float functions: integers become `float`.
        for (a, ty) in args.iter().zip(tys.iter_mut()) {
            if ty.is_int() {
                self.coerce(&a.value, *ty, Ty::Float);
                *ty = Ty::Float;
            }
        }
        if tys.contains(&Ty::Error) {
            return Ty::Error;
        }
        self.meet_args(e, b, args, &tys)
    }

    /// A lane-by-lane call with at least one vector among `tys`: every vector has the first one's
    /// type, every number fills its lanes, and the result is that type.
    fn check_lane_values(&mut self, e: &Expr, b: Builtin, args: &[Arg], tys: &[Ty]) -> Ty {
        if tys.contains(&Ty::Error) {
            return Ty::Error;
        }
        let vt = *tys.iter().find(|t| self.t.vector(**t).is_some()).expect("a vector");
        let kind = self.t.vector(vt).expect("a vector");
        let name = b.name();
        if !kind.has_builtin(b) {
            let msg = format!("`{name}` needs floats; convert with `{}(...)` first", self.tn(kind.as_float()));
            self.error(e.span, msg);
            return Ty::Error;
        }
        let mut ok = true;
        for (a, &ty) in args.iter().zip(tys) {
            if self.t.vector(ty).is_none() {
                ok &= self.fill_lanes(e.span, name, vt, kind, &a.value, ty);
            } else if ty != vt {
                let msg = format!("`{name}` can't mix {} and {}", self.tn(vt), self.tn(ty));
                self.error(e.span, msg);
                return Ty::Error;
            }
        }
        if !ok {
            return Ty::Error;
        }
        self.t.res.insert(e.id, Res::Builtin(b));
        vt
    }

    /// `lerp(a, b, t)` with a vector: `a` and `b` are float vectors of one type, or one of them a
    /// number that fills its lanes; `t` is a number (`tys` are the three values' types, `t`'s
    /// already required to be a number).
    pub(super) fn check_vec_lerp(&mut self, e: &Expr, args: &[Arg], tys: [Ty; 3]) -> Ty {
        let [first, second, t] = tys;
        if first == Ty::Error || second == Ty::Error {
            return Ty::Error;
        }
        let vt = if self.t.vector(first).is_some() { first } else { second };
        let kind = self.t.vector(vt).expect("a vector");
        if !kind.has_builtin(Builtin::Lerp) {
            let msg = format!("`lerp` needs floats; convert with `{}(...)` first", self.tn(kind.as_float()));
            self.error(e.span, msg);
            return Ty::Error;
        }
        let fits = |ty: Ty| ty == vt || (self.t.vector(ty).is_none() && ty.is_numeric());
        if !fits(first) || !fits(second) {
            self.lerp_mismatch(e, first, second);
            return Ty::Error;
        }
        if t == Ty::Error {
            return Ty::Error;
        }
        for (a, ty) in args.iter().zip([first, second]) {
            if ty != vt && !self.fill_lanes(e.span, "lerp", vt, kind, &a.value, ty) {
                return Ty::Error;
            }
        }
        self.coerce(&args[2].value, t, Ty::Float);
        self.t.res.insert(e.id, Res::Builtin(Builtin::Lerp));
        vt
    }

    /// A method of a vector (`length`, `normalized`, `dot`, `distance_to`, `cross`, `sum`). Its
    /// values are vectors of the receiver's type.
    pub(crate) fn check_vector_method(&mut self, e: &Expr, b: Builtin, receiver: &Expr, receiver_ty: Ty, args: &[Arg]) -> Ty {
        if let Some((span, msg)) = args_problem(e, b, args) {
            self.error(span, msg);
            self.check_loose_args(args);
            return Ty::Error;
        }
        let kind = self.t.vector(receiver_ty).expect("a vector receiver");
        let name = b.name();
        if b != Builtin::Sum && !kind.has_float_math() {
            let msg = format!("`{name}` needs a float vector; convert with `{}(...)` first", self.tn(kind.as_float()));
            self.error(e.span, msg);
            self.check_loose_args(args);
            return Ty::Error;
        }
        for a in args {
            self.check_value(&a.value, Some(receiver_ty));
        }
        if b == Builtin::Cross && kind.dims != 3 {
            let msg = format!("`cross` needs a vec3; {} has no cross product", self.tn(receiver_ty));
            self.error(e.span, msg);
            return Ty::Error;
        }
        // A receiver that is a plain place (`p.pos`) is read after the values
        // are computed, so they must not change it. (One that computes an
        // index, like `vs[i]`, is copied first, in order.)
        let later: Vec<&Expr> = args.iter().map(|a| &a.value).collect();
        self.check_changed_later(&[receiver], &later);
        self.t.res.insert(e.id, Res::Builtin(b));
        match b {
            Builtin::Normalized | Builtin::Cross => receiver_ty,
            Builtin::Sum => kind.lane(),
            _ => Ty::Float,
        }
    }
}
