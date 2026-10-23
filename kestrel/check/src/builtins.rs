//! The built-in functions and constants: GDScript-style global math and
//! random numbers, plus the vector methods.

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Builtin {
    Sqrt,
    Sin,
    Cos,
    Tan,
    Asin,
    Acos,
    Atan,
    Atan2,
    Exp,
    Log,
    Pow,
    Floor,
    Ceil,
    Round,
    Abs,
    Sign,
    Min,
    Max,
    Clamp,
    Lerp,
    Randf,
    RandfRange,
    RandiRange,
    RandSeed,
    Pi,
    Tau,
    /// Methods of the float vectors.
    Length,
    Normalized,
    Dot,
    DistanceTo,
    /// `vec3` only.
    Cross,
    /// `v.sum()` on every vector: its components added left to right.
    Sum,
}

/// Every global function and constant. A new variant belongs here or in `METHODS`.
const GLOBALS: [Builtin; 26] = [
    Builtin::Sqrt,
    Builtin::Sin,
    Builtin::Cos,
    Builtin::Tan,
    Builtin::Asin,
    Builtin::Acos,
    Builtin::Atan,
    Builtin::Atan2,
    Builtin::Exp,
    Builtin::Log,
    Builtin::Pow,
    Builtin::Floor,
    Builtin::Ceil,
    Builtin::Round,
    Builtin::Abs,
    Builtin::Sign,
    Builtin::Min,
    Builtin::Max,
    Builtin::Clamp,
    Builtin::Lerp,
    Builtin::Randf,
    Builtin::RandfRange,
    Builtin::RandiRange,
    Builtin::RandSeed,
    Builtin::Pi,
    Builtin::Tau,
];

const METHODS: [Builtin; 6] = [Builtin::Length, Builtin::Normalized, Builtin::Dot, Builtin::DistanceTo, Builtin::Cross, Builtin::Sum];

const GLOBAL_NAME_LIST: [&str; GLOBALS.len()] = {
    let mut names = [""; GLOBALS.len()];
    let mut i = 0;
    while i < GLOBALS.len() {
        names[i] = GLOBALS[i].name();
        i += 1;
    }
    names
};

impl Builtin {
    /// Every global name, for `BUILTINS`.
    pub const GLOBAL_NAMES: &'static [&'static str] = &GLOBAL_NAME_LIST;

    /// A global function or constant by name (not the vector methods).
    pub fn global(name: &str) -> Option<Builtin> {
        GLOBALS.into_iter().find(|b| b.name() == name)
    }

    /// A vector method by name.
    pub fn vector_method(name: &str) -> Option<Builtin> {
        METHODS.into_iter().find(|b| b.name() == name)
    }

    pub const fn name(self) -> &'static str {
        match self {
            Builtin::Sqrt => "sqrt",
            Builtin::Sin => "sin",
            Builtin::Cos => "cos",
            Builtin::Tan => "tan",
            Builtin::Asin => "asin",
            Builtin::Acos => "acos",
            Builtin::Atan => "atan",
            Builtin::Atan2 => "atan2",
            Builtin::Exp => "exp",
            Builtin::Log => "log",
            Builtin::Pow => "pow",
            Builtin::Floor => "floor",
            Builtin::Ceil => "ceil",
            Builtin::Round => "round",
            Builtin::Abs => "abs",
            Builtin::Sign => "sign",
            Builtin::Min => "min",
            Builtin::Max => "max",
            Builtin::Clamp => "clamp",
            Builtin::Lerp => "lerp",
            Builtin::Randf => "randf",
            Builtin::RandfRange => "randf_range",
            Builtin::RandiRange => "randi_range",
            Builtin::RandSeed => "rand_seed",
            Builtin::Pi => "PI",
            Builtin::Tau => "TAU",
            Builtin::Length => "length",
            Builtin::Normalized => "normalized",
            Builtin::Dot => "dot",
            Builtin::DistanceTo => "distance_to",
            Builtin::Cross => "cross",
            Builtin::Sum => "sum",
        }
    }

    /// `PI` and `TAU`: names, not calls.
    pub fn is_constant(self) -> bool {
        matches!(self, Builtin::Pi | Builtin::Tau)
    }

    /// False only for the four random ones: they read and change the generator.
    pub fn is_pure(self) -> bool {
        !matches!(self, Builtin::Randf | Builtin::RandfRange | Builtin::RandiRange | Builtin::RandSeed)
    }

    /// How many values a call takes (a method's receiver doesn't count).
    pub fn arity(self) -> usize {
        use Builtin::*;
        match self {
            Pi | Tau | Randf | Length | Normalized | Sum => 0,
            Sqrt | Sin | Cos | Tan | Asin | Acos | Atan | Exp | Log | Floor | Ceil | Round | Abs | Sign | RandSeed | Dot
            | DistanceTo | Cross => 1,
            Atan2 | Pow | Min | Max | RandfRange | RandiRange => 2,
            Clamp | Lerp => 3,
        }
    }
}
