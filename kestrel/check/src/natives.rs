//! The types a `native` function may use. Rust reads its arguments straight
//! from memory, so they are limited to values with a plain layout.

use kestrel_syntax::ast::{Arg, BinaryOp, Expr, ExprKind, Program};
use kestrel_syntax::Diagnostic;

use crate::tables::{FnInfo, ListMethod, Res, Tables};
use crate::ty::Ty;

const BAD_TYPE: &str = "native functions can only take numbers, bools, strings, vectors, arrays of plain values, and enums or structs of those";
const BAD_TYPE_HELP: &str = concat!(
    "a native can take int, i64, u8, float, f64, bool, string, vec2, vec3, an enum with no fields, ",
    "an array `[T]` of numbers, bools or structs of those, or a struct of all of these (a struct field ",
    "can also be a `list[T]` of those, but not an enum); it can return nothing, a number, a bool, a ",
    "vector, or a struct of numbers, bools and vectors"
);

/// Whether a call to `f` with these arguments builds its text in a scratch arena that is
/// freed when the call returns: `f` is a native, which only reads its arguments during the
/// call, and some argument directly builds a new string: an interpolated string, a string `+`,
/// a string method that gives a string (`s[i]` included) or a list's `join`. So
/// `draw.text("fps {fps}", ...)` or `draw.text(name + ": " + hp, ...)` in a frame loop frees
/// its text every frame instead of growing the arena it runs in. Text kept in a variable first
/// is not covered.
///
/// It needs the arguments' types, so it is asked once they are checked: KIR lowering asks it
/// to push and pop the arena. The checker, which treats the call's arguments as a region that
/// is freed at the call's end (like an `arena { }` block), has to decide before checking them,
/// so it asks `may_want_scratch`, which says yes whenever this may.
pub fn wants_scratch(f: &FnInfo, args: &[Arg], t: &Tables) -> bool {
    f.is_native && args.iter().any(|a| builds_text(&a.value, t))
}

/// Whether `wants_scratch` may say yes once the arguments are checked: `f` is a native and some
/// argument has the shape of one that builds a string. Saying yes when `wants_scratch` then
/// doesn't only makes the checker stricter about memory built inside the arguments.
pub(crate) fn may_want_scratch(f: &FnInfo, args: &[Arg]) -> bool {
    let shaped = |e: &Expr| {
        matches!(
            e.kind,
            ExprKind::Interp(_) | ExprKind::Binary { op: BinaryOp::Add, .. } | ExprKind::MethodCall { .. } | ExprKind::Index { .. }
        )
    };
    f.is_native && args.iter().any(|a| shaped(&a.value))
}

/// Whether checked expression `e` builds a new string (see `wants_scratch`).
fn builds_text(e: &Expr, t: &Tables) -> bool {
    match &e.kind {
        ExprKind::Interp(_) => true,
        _ if t.ty(e.id) != Ty::Str => false,
        ExprKind::Binary { op: BinaryOp::Add, .. } => true,
        ExprKind::MethodCall { .. } | ExprKind::Index { .. } => {
            matches!(t.res.get(&e.id), Some(Res::Str(_) | Res::List(ListMethod::Join)))
        }
        _ => false,
    }
}

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
            if !result_ok(t, f.ret) {
                diags.push(Diagnostic::error(ret.span, BAD_TYPE).with_help(BAD_TYPE_HELP));
            }
        }
    }
}

/// A type with a plain layout inside a struct. `Error` was already reported.
fn simple(ty: Ty) -> bool {
    matches!(ty, Ty::Int | Ty::I64 | Ty::U8 | Ty::Float | Ty::F64 | Ty::Bool | Ty::Str | Ty::Error)
}

/// A value a native can read straight from an array's memory: a number, a bool,
/// or a struct of those (vectors and colours are structs of floats).
fn plain(t: &Tables, ty: Ty) -> bool {
    plain_below(t, ty, &mut Vec::new())
}

/// `plain`, skipping the structs in `seen`: a struct that contains itself is an error
/// reported elsewhere, and looking into it again would never end.
fn plain_below(t: &Tables, ty: Ty, seen: &mut Vec<Ty>) -> bool {
    match ty {
        Ty::Int | Ty::I64 | Ty::U8 | Ty::Float | Ty::F64 | Ty::Bool | Ty::Error => true,
        Ty::Struct(_) if seen.contains(&ty) => true,
        Ty::Struct(s) => {
            seen.push(ty);
            t.strukt(s).fields.iter().all(|f| plain_below(t, f.ty, seen))
        }
        _ => false,
    }
}

/// `[T]` or `list[T]` (as `list` allows) of a plain `T`.
fn plain_seq(t: &Tables, ty: Ty, list: bool) -> bool {
    match ty {
        Ty::Array(e) => plain(t, t.elem(e)),
        Ty::List(e) => list && plain(t, t.elem(e)),
        _ => false,
    }
}

fn param_ok(t: &Tables, ty: Ty) -> bool {
    match ty {
        Ty::Struct(s) => {
            // A `GpuHandle` field is only usable in a built-in module's structs (`resolve_type` refuses it elsewhere).
            t.strukt(s).fields.iter().all(|f| simple(f.ty) || f.ty == Ty::Gpu || plain(t, f.ty) || plain_seq(t, f.ty, true))
        }
        Ty::Enum(e) => t.enm(e).variants.iter().all(|v| v.fields.is_empty()),
        Ty::Array(_) => plain_seq(t, ty, false),
        other => simple(other),
    }
}

fn result_ok(t: &Tables, ty: Ty) -> bool {
    match ty {
        // A built-in module's struct that holds a GPU object, like `Texture`.
        Ty::Struct(s) => t.strukt(s).fields.iter().all(|f| plain(t, f.ty) || f.ty == Ty::Gpu),
        other => simple(other) && other != Ty::Str,
    }
}
