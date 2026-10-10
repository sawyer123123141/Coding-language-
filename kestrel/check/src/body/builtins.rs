//! Typing for the global built-ins: `sqrt(x)`, `atan2(y, x)`, `PI`...

use kestrel_syntax::ast::{Arg, Expr};

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
        } else if !is_float_function(b) {
            // Typed by a later task.
            Some((e.span, format!("`{name}` isn't available yet")))
        } else if let Some(bad) = args.iter().find(|a| a.label.is_some() || a.is_var) {
            Some((bad.span, "built-in functions take plain values, not labels or `var`".to_string()))
        } else if args.len() != b.arity() {
            Some((e.span, format!("`{name}` takes {}", count_words(b.arity()))))
        } else {
            None
        };
        if let Some((span, msg)) = problem {
            self.error(span, msg);
            // Mistakes inside the arguments still show.
            self.check_loose_args(args);
            return Ty::Error;
        }
        self.check_float_args(e, b, args)
    }

    /// The float functions: every argument is a number, integers become
    /// `float`, and the arguments meet at one float type, which is the result's.
    fn check_float_args(&mut self, e: &Expr, b: Builtin, args: &[Arg]) -> Ty {
        let mut tys = Vec::with_capacity(args.len());
        for a in args {
            let ty = self.check_expr(&a.value, None);
            let mut ty = self.reject_never(&a.value, ty);
            if ty.is_int() {
                self.coerce(&a.value, ty, Ty::Float);
                ty = Ty::Float;
            } else if !ty.is_float() && ty != Ty::Error {
                let msg = format!("`{}` needs a number here, found {}", b.name(), self.tn(ty));
                self.error(a.value.span, msg);
                ty = Ty::Error;
            }
            tys.push(ty);
        }
        if tys.contains(&Ty::Error) {
            return Ty::Error;
        }
        let result = tys.iter().copied().reduce(|x, y| Ty::common(x, y).expect("floats meet at the wider one")).unwrap_or(Ty::Float);
        for (a, &ty) in args.iter().zip(&tys) {
            self.coerce(&a.value, ty, result);
        }
        self.t.res.insert(e.id, Res::Builtin(b));
        result
    }
}
