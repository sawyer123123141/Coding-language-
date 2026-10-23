//! Lanes and swizzles. One component of a `vec4` (`v.x`, `p.pos.w`) is a lane, recorded in
//! `Tables::lanes`: a place when its vector is one, as a field is. (One component of a struct
//! vector, `v2.x`, is a field.) Several components (`v.xz`, `iv.zyx`, `v3.xyzz`) are a swizzle: a
//! new vector of the same kind.

use kestrel_syntax::ast::Expr;
use kestrel_syntax::Span;

use super::BodyChecker;
use crate::ty::Ty;
use crate::vectors::{vector_ty, COMPONENTS};

impl<'a> BodyChecker<'a> {
    /// `v.xz`, `iv.zyx`: the components `name` picks from a vector of type `base_ty` (repeats
    /// allowed), and the vector they make: as many components, of the same kind (an int vector
    /// gives an int vector).
    pub(super) fn swizzle(&self, base_ty: Ty, name: &str) -> Option<(Vec<u32>, Ty)> {
        let kind = self.t.vector(base_ty)?;
        let comps: Option<Vec<u32>> = name
            .chars()
            .map(|c| COMPONENTS[..kind.dims as usize].iter().position(|n| n.starts_with(c)).map(|i| i as u32))
            .collect();
        let comps = comps?;
        if comps.len() < 2 {
            return None;
        }
        let ty = vector_ty(comps.len() as u32, kind.int)?;
        Some((comps, ty))
    }

    /// Whether the checked field access `e` is a field of a struct or a lane of a `vec4`: a place
    /// when its base is one. (A swizzle, `.len` or an enum variant is neither.)
    pub(super) fn is_field_or_lane(&self, e: &Expr) -> bool {
        self.t.field_index.contains_key(&e.id) || self.t.lanes.contains_key(&e.id)
    }

    /// `e`, which is `name` of a SIMD vector of type `base_ty`: a lane (`v.w`) or a swizzle.
    pub(super) fn simd_lane(&mut self, e: &Expr, base_ty: Ty, name: &str, name_span: Span) -> Ty {
        let kind = self.t.vector(base_ty).expect("a vector");
        if let Some(i) = COMPONENTS[..kind.dims as usize].iter().position(|&c| c == name) {
            self.t.lanes.insert(e.id, i as u32);
            return kind.lane();
        }
        if let Some((comps, ty)) = self.swizzle(base_ty, name) {
            self.t.swizzles.insert(e.id, comps);
            return ty;
        }
        let msg = format!("`{}` has no field `{name}`", self.tn(base_ty));
        self.error(name_span, msg);
        Ty::Error
    }
}
