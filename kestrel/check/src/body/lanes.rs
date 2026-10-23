//! Swizzles: `v.xz`, `iv.zyx`, several components of a vector read as a new vector of the same
//! kind. (One component, `v.x`, is a field.)

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
}
