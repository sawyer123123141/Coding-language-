mod body;
mod builtins;
mod collect;
mod closure;
mod collect_ty;
pub mod generic;
mod iface;
mod list_method;
mod natives;
mod sig;
mod str_method;
pub mod tables;
mod tables_ty;
pub mod ty;
mod vectors;

use kestrel_syntax::ast::Program;
use kestrel_syntax::Diagnostic;

pub use builtins::Builtin;
pub use generic::{CopyInfo, GenericOwner, Generics, Instance, ParamId, ParamInfo, Subst, TyListId};
pub use collect::is_reserved_name;
pub use natives::wants_scratch;
pub use tables::{
    ConstId, ConstInfo, EnumId, EnumInfo, FieldInfo, FnId, FnInfo, IfaceId, IfaceInfo, ListMethod, LocalId, LocalInfo, ParamSig, Res, StrMethod, StructId,
    StructInfo, Tables, VariantInfo, BUILTIN_STRUCTS, IVEC2, IVEC3, VEC2, VEC3,
};
pub use vectors::{vec_kind, vector_ty, VecKind};
pub use closure::{Capture, ClosureInfo};
pub use sig::{sig_accepts, Sig, SigId, SigParam};
pub use iface::implements;
pub use ty::Ty;

/// Resolves names and checks types. Tables are returned only when there
/// are no errors; warnings may still be present.
pub fn check(program: &Program) -> (Option<Tables>, Vec<Diagnostic>) {
    check_with(program, true)
}

/// Like `check`; with `needs_main` false the program is a library (code a
/// host calls, like the Godot add-on), which doesn't need a `main`.
pub fn check_with(program: &Program, needs_main: bool) -> (Option<Tables>, Vec<Diagnostic>) {
    let (t, diags) = check_keeping_tables(program, needs_main);
    let ok = !diags.iter().any(|d| d.is_error());
    (ok.then_some(t), diags)
}

/// Like `check_with`, but the tables come back even when there are errors, for an editor: the
/// code with no errors still has its types and names resolved; an expression with an error has
/// the type `Ty::Error`. Nothing may be lowered from tables that came with errors.
pub fn check_keeping_tables(program: &Program, needs_main: bool) -> (Tables, Vec<Diagnostic>) {
    let mut t = Tables {
        expr_ty: vec![Ty::Error; program.expr_count as usize],
        builtin_ranges: program.builtin_ranges.clone(),
        hex_literals: program.hex_literals.iter().copied().collect(),
        ..Tables::default()
    };
    let mut diags = Vec::new();
    collect::collect(program, &mut t, &mut diags, needs_main);
    t.compute_owning();
    body::check_consts(program, &mut t, &mut diags);
    body::check_field_defaults(program, &mut t, &mut diags);
    // Defaults that build each other would recurse forever in KIR; this needs a clean check so far.
    if !diags.iter().any(|d| d.is_error()) {
        body::check_default_cycles(program, &t, &mut diags);
    }
    for f in 0..t.fns.len() {
        // Temporary (Task 2a to Task 3a, P12): generic functions and methods of generic types are
        // checked once their bodies know their placeholders (`BodyChecker`'s type scope).
        if t.fns[f].is_abstract || t.fns[f].generic.is_some() {
            continue;
        }
        body::check_fn(program, FnId(f as u32), &mut t, &mut diags);
    }
    collect_ty::check_map_values(&t, &mut diags);
    (t, diags)
}
