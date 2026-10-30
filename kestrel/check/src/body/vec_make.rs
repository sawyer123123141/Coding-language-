//! Making a vector (syntax pick 3, the same for every vector type): one number per component
//! (`vec3(1, 2, 3)`, or labelled like a struct's fields), none for zeros (`vec3()`), one number
//! for all of them (`vec3(0.5)`), one vector to copy or convert (`vec3(v)`, `vec3(iv)`), or numbers
//! and vectors whose components add up to the vector's, in order (`vec3(v2, 1)`, the shader rule).

use kestrel_syntax::ast::{Arg, Expr};

use super::BodyChecker;
use crate::tables::Res;
use crate::ty::Ty;
use crate::vectors::{vec_kind, vector_name, VecKind, COMPONENTS};

impl<'a> BodyChecker<'a> {
    /// A call of a vector type's name (`vt`). The values are checked first, in source order, to tell
    /// the forms apart; one number per component (labelled or not) and none bind like a struct's
    /// fields (`check_construct_with`, or `simd_lanes` for a SIMD vector), the rest are recorded in
    /// `Tables::vec_makes`.
    pub(crate) fn check_vec_make(&mut self, e: &Expr, vt: Ty, args: &[Arg]) -> Ty {
        let kind = self.t.vector(vt).expect("a vector type's name");
        let tys: Vec<Ty> = args.iter().map(|a| self.check_expr(&a.value, Some(kind.lane()))).collect();
        let res = match vt {
            Ty::Struct(sid) => Res::Construct(sid),
            _ => Res::SimdMake(vt),
        };
        self.t.res.insert(e.id, res);
        if tys.contains(&Ty::Error) {
            return vt;
        }
        let is_vector = |ty: &Ty| vec_kind(*ty).is_some();
        let labelled = args.iter().any(|a| a.label.is_some());
        // V4 is tested first, among all the values: it wins over how the labels bind.
        if labelled && tys.iter().any(is_vector) {
            self.error(e.span, "labels only work when every component is a number, like `vec4(x: 1, y: 2, z: 3, w: 4)`");
            return vt;
        }
        if !self.int_numbers(vt, kind, args, &tys) {
            return vt;
        }
        let per_component = !tys.iter().any(is_vector) && args.len() == kind.dims as usize;
        if labelled || args.is_empty() || per_component {
            return match vt {
                Ty::Struct(sid) => self.check_construct_with(e, sid, args, Some(&tys)),
                _ => self.simd_lanes(e, vt, kind, args, &tys),
            };
        }
        if self.vec_parts(e, vt, kind, args, &tys) {
            self.t.vec_makes.insert(e.id);
        }
        vt
    }

    /// `vec4(1, 2, 3, 4)`, `vec4(w: 4, z: 3, y: 2, x: 1)`, `vec4()`: the numbers (already checked,
    /// as `tys`) bind to the lanes like arguments to parameters, positional first, then labels; none
    /// at all is zeros. Records the binding in `ctor_args`, one entry per lane.
    fn simd_lanes(&mut self, e: &Expr, vt: Ty, kind: VecKind, args: &[Arg], tys: &[Ty]) -> Ty {
        let name = self.tn(vt);
        let lanes = &COMPONENTS[..kind.dims as usize];
        let (slots, mut ok) = self.bind_args(&name, lanes, args, "field");
        // In source order, the order they run in.
        for (ai, a) in args.iter().enumerate() {
            if a.is_var {
                self.error(a.span, "constructor arguments can't be `var`");
                ok = false;
            }
            if slots.contains(&Some(ai)) {
                self.finish_value(&a.value, tys[ai], Some(kind.lane()));
            }
            self.consume(&a.value);
        }
        if ok && !args.is_empty() {
            let missing: Vec<String> = slots.iter().zip(lanes).filter(|(s, _)| s.is_none()).map(|(_, l)| format!("`{l}`")).collect();
            ok = self.missing_fields(e, &name, &missing, true);
        }
        if ok {
            self.t.ctor_args.insert(e.id, slots);
        }
        vt
    }

    /// V2: an int vector takes `int` and `u8` numbers only. Returns whether every number is one.
    fn int_numbers(&mut self, vt: Ty, kind: VecKind, args: &[Arg], tys: &[Ty]) -> bool {
        if !kind.int {
            return true;
        }
        let mut ok = true;
        for (a, &ty) in args.iter().zip(tys) {
            if matches!(ty, Ty::Float | Ty::F64 | Ty::I64) {
                // From an `i64` the conversion stops the program when the value doesn't fit.
                let traps = if ty == Ty::I64 { ", which in a normal build stops the program if the value doesn't fit in `int`" } else { "" };
                let msg = format!("`{}` holds ints, found {}; convert it with `int(...)` first{traps}", self.tn(vt), self.tn(ty));
                self.error(a.value.span, msg);
                ok = false;
            }
        }
        ok
    }

    /// A fill, a copy, a conversion or a mixed make: each number becomes one component (or, alone,
    /// all of them), each vector of the same kind gives its components, and one vector of the
    /// other kind and the same size converts. Returns whether it is right (V1, V3). When the
    /// components don't add up, that is the one error (V1): converting a value (V3's advice)
    /// wouldn't fix it (`ivec3(vec4(1))`).
    fn vec_parts(&mut self, e: &Expr, vt: Ty, kind: VecKind, args: &[Arg], tys: &[Ty]) -> bool {
        let mut ok = true;
        let mut count = 0;
        let mut kinds = Vec::new();
        for (a, &ty) in args.iter().zip(tys) {
            if a.is_var {
                self.error(a.span, "constructor arguments can't be `var`");
                ok = false;
            }
            let ty = self.reject_never(&a.value, ty);
            match self.t.vector(ty) {
                Some(k) => {
                    let converts = args.len() == 1 && k.dims == kind.dims;
                    if k.int != kind.int && !converts {
                        let what = if kind.int { "ints and int vectors" } else { "floats and float vectors" };
                        let wanted = vector_name(k.dims, kind.int);
                        let msg = format!("`{}` takes {what}, found {}; convert it with `{wanted}(...)` first", self.tn(vt), self.tn(ty));
                        kinds.push((a.value.span, msg));
                    }
                    count += k.dims;
                }
                None => {
                    self.coerce(&a.value, ty, kind.lane());
                    count += 1;
                }
            }
            self.consume(&a.value);
        }
        let fill = args.len() == 1 && count == 1;
        if ok && !fill && count != kind.dims {
            let n = kind.dims;
            let msg = format!("`{}` has {n} components, but these values give {count}; give {n}, one value for all of them, or none for zeros", self.tn(vt));
            self.error(e.span, msg);
            return false;
        }
        for (span, msg) in kinds {
            self.error(span, msg);
            ok = false;
        }
        ok
    }
}
