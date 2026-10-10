//! The types a `native` function may use. Rust reads its arguments straight
//! from memory, so they are limited to values with a plain layout.

use kestrel_syntax::ast::Program;
use kestrel_syntax::Diagnostic;

use crate::tables::{Tables, VEC2, VEC3};
use crate::ty::Ty;

const BAD_TYPE: &str = "native functions can only take numbers, bools, strings, vectors, and enums or structs of those";
const BAD_TYPE_HELP: &str = concat!(
    "use int, i64, u8, float, f64, bool, string, vec2, vec3, an enum with no fields, or a struct of those; ",
    "a native may return only nothing, a number, a bool, vec2 or vec3"
);

/// Checks every native's parameters and result. Run once all types are
/// collected, since a native may name a struct declared after it.
pub(crate) fn check_natives(program: &Program, t: &Tables, diags: &mut Vec<Diagnostic>) {
    for f in t.fns.iter().filter(|f| f.is_native) {
        let decl = f.decl(program);
        for (p, sig) in decl.params.iter().zip(&f.params) {
            if p.is_var {
                let help = "natives only read their arguments; remove `var`";
                diags.push(Diagnostic::error(p.bind.span, "native functions can't take `var` parameters").with_help(help));
            }
            if !param_ok(t, sig.ty) {
                diags.push(Diagnostic::error(p.ty.span, BAD_TYPE).with_help(BAD_TYPE_HELP));
            }
        }
        if let Some(ret) = &decl.ret {
            if !result_ok(f.ret) {
                diags.push(Diagnostic::error(ret.span, BAD_TYPE).with_help(BAD_TYPE_HELP));
            }
        }
    }
}

/// A type with a plain layout inside a struct. `Error` was already reported.
fn simple(ty: Ty) -> bool {
    matches!(ty, Ty::Int | Ty::I64 | Ty::U8 | Ty::Float | Ty::F64 | Ty::Bool | Ty::Str | Ty::Error)
}

fn param_ok(t: &Tables, ty: Ty) -> bool {
    match ty {
        Ty::Struct(s) => t.strukt(s).fields.iter().all(|f| simple(f.ty) || matches!(f.ty, Ty::Struct(VEC2 | VEC3))),
        Ty::Enum(e) => t.enm(e).variants.iter().all(|v| v.fields.is_empty()),
        other => simple(other),
    }
}

fn result_ok(ty: Ty) -> bool {
    matches!(ty, Ty::Struct(VEC2 | VEC3)) || (simple(ty) && ty != Ty::Str)
}
