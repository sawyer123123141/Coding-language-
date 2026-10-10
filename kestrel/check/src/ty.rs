/// A checked type. `Never` is the type of code that doesn't finish
/// (`return`, `break`); `Error` suppresses follow-on errors.
use crate::tables::{ElemId, EnumId, StructId};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Ty {
    Unit,
    Never,
    Bool,
    Int,
    I64,
    U8,
    Float,
    F64,
    Str,
    /// A user-defined struct; its name lives in `Tables::structs`.
    Struct(StructId),
    Enum(EnumId),
    /// A `match` binding: the fields of one variant of the matched value.
    /// Only usable as `binding.field`.
    Variant(EnumId, u32),
    /// `[T]`: fixed length once created. The element type is in `Tables::elems`.
    Array(ElemId),
    /// `list[T]`: growable.
    List(ElemId),
    /// `map[K, V]`: keys and values, in insertion order.
    Map(ElemId, ElemId),
    /// A memory bucket. Values holding one are owning: they move, never copy.
    Arena,
    /// `T?`: a `T` or `none`.
    Optional(ElemId),
    Error,
}

impl Ty {
    pub fn from_name(name: &str) -> Option<Ty> {
        Some(match name {
            "int" => Ty::Int,
            "i64" => Ty::I64,
            "u8" => Ty::U8,
            "float" => Ty::Float,
            "f64" => Ty::F64,
            "bool" => Ty::Bool,
            "string" => Ty::Str,
            "Arena" => Ty::Arena,
            _ => return None,
        })
    }

    pub fn name(self) -> &'static str {
        match self {
            Ty::Unit => "nothing",
            Ty::Never => "never",
            Ty::Bool => "bool",
            Ty::Int => "int",
            Ty::I64 => "i64",
            Ty::U8 => "u8",
            Ty::Float => "float",
            Ty::F64 => "f64",
            Ty::Str => "string",
            Ty::Struct(_) => "struct",
            Ty::Enum(_) => "enum",
            Ty::Variant(..) => "variant",
            Ty::Array(_) => "array",
            Ty::List(_) => "list",
            Ty::Map(..) => "map",
            Ty::Arena => "Arena",
            Ty::Optional(_) => "optional",
            Ty::Error => "{error}",
        }
    }

    pub fn is_int(self) -> bool {
        matches!(self, Ty::Int | Ty::I64 | Ty::U8)
    }

    pub fn is_float(self) -> bool {
        matches!(self, Ty::Float | Ty::F64)
    }

    pub fn is_numeric(self) -> bool {
        self.is_int() || self.is_float()
    }

    /// Values that live in memory and are passed by pointer.
    pub fn is_aggregate(self) -> bool {
        matches!(
            self,
            Ty::Struct(_) | Ty::Enum(_) | Ty::Str | Ty::Array(_) | Ty::List(_) | Ty::Map(..) | Ty::Arena | Ty::Optional(_)
        )
    }

    /// Whether a value of `self` may be used where `to` is expected without
    /// an explicit conversion: int→float, narrower→wider int, float→f64.
    pub fn coerces_to(self, to: Ty) -> bool {
        use Ty::*;
        self == to
            || matches!(self, Never | Error)
            || to == Error
            || matches!(
                (self, to),
                (U8, Int) | (U8, I64) | (Int, I64) | (U8 | Int | I64, Float | F64) | (Float, F64)
            )
    }

    /// The type two operands meet at, if one converts to the other.
    pub fn common(a: Ty, b: Ty) -> Option<Ty> {
        if a.coerces_to(b) {
            Some(b)
        } else if b.coerces_to(a) {
            Some(a)
        } else {
            None
        }
    }
}
