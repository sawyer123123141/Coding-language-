//! The one table of the vector types and what each has: one rule for every vector (syntax pick 4).
//! A vector type is a row here; its operators, built-ins and methods follow from its kind.

use crate::builtins::Builtin;
use crate::tables::{Tables, IVEC2, IVEC3, VEC2, VEC3};
use crate::ty::Ty;

/// What kind of vector a type is: how many components, whether they are `int`s (else `float`s),
/// and whether it is one SIMD value (else a struct that scalar replacement splits into numbers).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VecKind {
    pub dims: u32,
    pub int: bool,
    pub simd: bool,
}

/// Every vector type, with its kind.
const VECTORS: [(Ty, VecKind); 6] = [
    (Ty::Struct(VEC2), VecKind { dims: 2, int: false, simd: false }),
    (Ty::Struct(VEC3), VecKind { dims: 3, int: false, simd: false }),
    (Ty::Vec4, VecKind { dims: 4, int: false, simd: true }),
    (Ty::Struct(IVEC2), VecKind { dims: 2, int: true, simd: false }),
    (Ty::Struct(IVEC3), VecKind { dims: 3, int: true, simd: false }),
    (Ty::IVec4, VecKind { dims: 4, int: true, simd: true }),
];

/// The name of the vector type with `dims` components of ints (`int`) or floats: `vec4`,
/// `ivec3`. (Every name is a vector type's: messages name them.)
pub fn vector_name(dims: u32, int: bool) -> String {
    format!("{}vec{dims}", if int { "i" } else { "" })
}

/// The component names, in order; a vector of `dims` components has the first `dims`.
pub const COMPONENTS: [&str; 4] = ["x", "y", "z", "w"];

/// The kind of `ty`, if it is a vector type.
pub fn vec_kind(ty: Ty) -> Option<VecKind> {
    VECTORS.iter().find(|(t, _)| *t == ty).map(|&(_, k)| k)
}

/// The vector type with `dims` components of ints (`int`) or floats, if there is one.
pub fn vector_ty(dims: u32, int: bool) -> Option<Ty> {
    VECTORS.iter().find(|(_, k)| k.dims == dims && k.int == int).map(|&(t, _)| t)
}

impl Tables {
    /// The kind of `ty`, if it is a vector type.
    pub fn vector(&self, ty: Ty) -> Option<VecKind> {
        vec_kind(ty)
    }
}

impl VecKind {
    /// The type of one component: `int` or `float`.
    pub fn lane(self) -> Ty {
        if self.int {
            Ty::Int
        } else {
            Ty::Float
        }
    }

    /// The vector of the same size with float components (`vec3` for `ivec3`), the one a
    /// conversion message names.
    pub fn as_float(self) -> Ty {
        vector_ty(self.dims, false).expect("every int vector has a float vector of its size")
    }

    /// `/`: every vector but `ivec4` (the CPU can't divide integer lanes at once).
    pub fn has_div(self) -> bool {
        !(self.int && self.simd)
    }

    /// `%`: `ivec2` and `ivec3`, component by component as `int` does.
    pub fn has_rem(self) -> bool {
        self.int && !self.simd
    }

    /// `& | ^ ~ << >>` and the wrapping operators: int vectors.
    pub fn has_bits(self) -> bool {
        self.int
    }

    /// The built-ins that only make sense on floats (`floor ceil round sqrt lerp`) and the
    /// methods `length normalized dot distance_to cross`.
    pub fn has_float_math(self) -> bool {
        !self.int
    }

    /// Whether the lane-by-lane built-in `b` works on this kind (`min max clamp abs` on all).
    pub fn has_builtin(self, b: Builtin) -> bool {
        match b {
            Builtin::Min | Builtin::Max | Builtin::Clamp | Builtin::Abs => true,
            _ => self.has_float_math(),
        }
    }
}

/// The global built-ins that work lane by lane on vectors (a plain number fills the lanes).
pub fn is_lane_builtin(b: Builtin) -> bool {
    use Builtin::*;
    matches!(b, Min | Max | Clamp | Abs | Floor | Ceil | Round | Sqrt | Lerp)
}
