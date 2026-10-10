//! Typing for the global built-ins: `sqrt(x)`, `atan2(y, x)`, `PI`,
//! `clamp(x, lo, hi)`... and the `vec2` / `vec3` methods.

use kestrel_syntax::ast::{Arg, Expr};
use kestrel_syntax::Span;

use super::BodyChecker;
use crate::builtins::Builtin;
use crate::tables::Res;
use crate::ty::Ty;

/// How a call's value count reads in "`sqrt` takes one value".
fn count_words(n: usize) -> &'static str {
    match n {
        0 => "no values",
        1 => "one value",
        2 => "two values",
        3 => "three values",
        _ => unreachable!("built-in functions take at most three values"),
    }
}

/// The functions that take floats and give a float (`sqrt`...`round`).
fn is_float_function(b: Builtin) -> bool {
    use Builtin::*;
    matches!(b, Sqrt | Sin | Cos | Tan | Asin | Acos | Atan | Atan2 | Exp | Log | Pow | Floor | Ceil | Round)
}

/// The functions that work on any number (`abs`...`lerp`).
fn is_generic_function(b: Builtin) -> bool {
    use Builtin::*;
    matches!(b, Abs | Sign | Min | Max | Clamp | Lerp)
}

/// What is wrong with the values written in a built-in call, if anything.
fn args_problem(e: &Expr, b: Builtin, args: &[Arg]) -> Option<(Span, String)> {
    if let Some(bad) = args.iter().find(|a| a.label.is_some() || a.is_var) {
        Some((bad.span, "built-in functions take plain values, not labels or `var`".to_string()))
    } else if args.len() != b.arity() {
        Some((e.span, format!("`{}` takes {}", b.name(), count_words(b.arity()))))
    } else {
        None
    }
}

impl<'a> BodyChecker<'a> {
    /// `PI` and `TAU`: floats, as wide as the context wants.
    pub(crate) fn builtin_constant(&mut self, e: &Expr, name: &str, want: Option<Ty>) -> Option<Ty> {
        let b = Builtin::global(name).filter(|b| b.is_constant())?;
        self.t.res.insert(e.id, Res::Builtin(b));
        Some(if want == Some(Ty::F64) { Ty::F64 } else { Ty::Float })
    }

    /// A call to a global built-in function.
    pub(crate) fn check_builtin_call(&mut self, e: &Expr, b: Builtin, args: &[Arg]) -> Ty {
        let name = b.name();
        let problem = if b.is_constant() {
            Some((e.span, format!("`{name}` is a constant; write it without `(...)`")))
        } else if !is_float_function(b) && !is_generic_function(b) {
            // Typed by a later task.
            Some((e.span, format!("`{name}` isn't available yet")))
        } else {
            args_problem(e, b, args)
        };
        if let Some((span, msg)) = problem {
            self.error(span, msg);
            // Mistakes inside the arguments still show.
            self.check_loose_args(args);
            return Ty::Error;
        }
        if is_float_function(b) {
            self.check_float_args(e, b, args)
        } else if b == Builtin::Lerp {
            self.check_lerp(e, args)
        } else {
            self.check_number_args(e, b, args)
        }
    }

    /// A call to a `vec2` / `vec3` method (`length`, `normalized`, `dot`, `distance_to`).
    /// Its values are vectors of the receiver's type.
    pub(crate) fn check_vector_method(&mut self, e: &Expr, b: Builtin, receiver: &Expr, receiver_ty: Ty, args: &[Arg]) -> Ty {
        if let Some((span, msg)) = args_problem(e, b, args) {
            self.error(span, msg);
            self.check_loose_args(args);
            return Ty::Error;
        }
        for a in args {
            self.check_value(&a.value, Some(receiver_ty));
        }
        // A receiver that is a plain place (`p.pos`) is read after the values
        // are computed, so they must not change it. (One that computes an
        // index, like `vs[i]`, is copied first, in order.)
        let later: Vec<&Expr> = args.iter().map(|a| &a.value).collect();
        self.check_changed_later(&[receiver], &later);
        self.t.res.insert(e.id, Res::Builtin(b));
        if b == Builtin::Normalized {
            receiver_ty
        } else {
            Ty::Float
        }
    }

    /// `ty`, the type of `e`, which a built-in needs to be a number.
    fn require_number(&mut self, b: Builtin, e: &Expr, ty: Ty) -> Ty {
        if ty.is_numeric() || ty == Ty::Error {
            return ty;
        }
        let msg = format!("`{}` needs a number here, found {}", b.name(), self.tn(ty));
        self.error(e.span, msg);
        Ty::Error
    }

    /// An argument that must be a number; its type (`Error` if it isn't one).
    /// A literal takes the type `hint` if it fits.
    fn number_arg(&mut self, b: Builtin, a: &Arg, hint: Option<Ty>) -> Ty {
        let ty = self.check_expr(&a.value, hint);
        let ty = self.reject_never(&a.value, ty);
        self.require_number(b, &a.value, ty)
    }

    /// Converts every argument to the type they all meet at, which is the result's.
    /// `tys` are their types, all numbers.
    fn meet_args(&mut self, e: &Expr, b: Builtin, args: &[Arg], tys: &[Ty]) -> Ty {
        let result = tys.iter().copied().reduce(|x, y| Ty::common(x, y).expect("numbers meet at one of them")).unwrap_or(Ty::Float);
        for (a, &ty) in args.iter().zip(tys) {
            self.coerce(&a.value, ty, result);
        }
        self.t.res.insert(e.id, Res::Builtin(b));
        result
    }

    /// The float functions: every argument is a number, integers become
    /// `float`, and the arguments meet at one float type, which is the result's.
    fn check_float_args(&mut self, e: &Expr, b: Builtin, args: &[Arg]) -> Ty {
        let mut tys = Vec::with_capacity(args.len());
        for a in args {
            let mut ty = self.number_arg(b, a, None);
            if ty.is_int() {
                self.coerce(&a.value, ty, Ty::Float);
                ty = Ty::Float;
            }
            tys.push(ty);
        }
        if tys.contains(&Ty::Error) {
            return Ty::Error;
        }
        self.meet_args(e, b, args, &tys)
    }

    /// `abs`, `sign`, `min`, `max`, `clamp`: every argument is a number, and
    /// they meet at one type (like `+`, where a literal takes the type of the
    /// value before it), which is the result's.
    fn check_number_args(&mut self, e: &Expr, b: Builtin, args: &[Arg]) -> Ty {
        let mut tys = Vec::with_capacity(args.len());
        let mut met: Option<Ty> = None;
        for a in args {
            let ty = self.number_arg(b, a, met);
            met = Some(met.and_then(|m| Ty::common(m, ty)).unwrap_or(ty)).filter(|t| t.is_numeric());
            tys.push(ty);
        }
        if tys.contains(&Ty::Error) {
            return Ty::Error;
        }
        self.meet_args(e, b, args, &tys)
    }

    /// `lerp(a, b, t)`: two numbers (the result is a float, even for integers)
    /// or two vectors of the same type, and a number `t` of the result's type.
    fn check_lerp(&mut self, e: &Expr, args: &[Arg]) -> Ty {
        let mut tys = [Ty::Error; 3];
        for (slot, a) in tys.iter_mut().zip(args) {
            let ty = self.check_expr(&a.value, None);
            *slot = self.reject_never(&a.value, ty);
        }
        let [first, second, t] = tys;
        let t = self.require_number(Builtin::Lerp, &args[2].value, t);
        let vector = self.t.vec_dims(first).is_some();
        let fits = if vector { second == first } else { first.is_numeric() && second.is_numeric() };
        if !fits {
            if first != Ty::Error && second != Ty::Error {
                let (a, b) = (self.tn(first), self.tn(second));
                let msg = format!("`lerp` needs two numbers or two vectors of the same size, found {a} and {b}");
                self.error(e.span, msg);
            }
            return Ty::Error;
        }
        if t == Ty::Error {
            return Ty::Error;
        }
        // Integers have no halfway points: they meet at `float`. Vector components are floats.
        let common = Ty::common(first, second).expect("numbers, and equal vectors, meet at one of them");
        let result = if !vector && common.is_int() { Ty::Float } else { common };
        let t_to = if vector { Ty::Float } else { result };
        self.coerce(&args[0].value, first, result);
        self.coerce(&args[1].value, second, result);
        self.coerce(&args[2].value, t, t_to);
        self.t.res.insert(e.id, Res::Builtin(Builtin::Lerp));
        result
    }
}
