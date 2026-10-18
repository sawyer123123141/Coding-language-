mod body;
mod builtins;
mod collect;
mod collect_ty;
mod natives;
mod sig;
mod str_method;
pub mod tables;
mod tables_ty;
pub mod ty;

use kestrel_syntax::ast::Program;
use kestrel_syntax::Diagnostic;

pub use builtins::Builtin;
pub use collect::is_reserved_name;
pub use natives::wants_scratch;
pub use tables::{
    ConstId, ConstInfo, EnumId, EnumInfo, FieldInfo, FnId, FnInfo, ListMethod, LocalId, LocalInfo, ParamSig, Res, StrMethod, StructId,
    StructInfo, Tables, VariantInfo, BUILTIN_STRUCTS, VEC2, VEC3,
};
pub use sig::{sig_accepts, Sig, SigId, SigParam};
pub use ty::Ty;

/// Resolves names and checks types. Tables are returned only when there
/// are no errors; warnings may still be present.
pub fn check(program: &Program) -> (Option<Tables>, Vec<Diagnostic>) {
    check_with(program, true)
}

/// Like `check`; with `needs_main` false the program is a library (code a
/// host calls, like the Godot add-on), which doesn't need a `main`.
pub fn check_with(program: &Program, needs_main: bool) -> (Option<Tables>, Vec<Diagnostic>) {
    let mut t = Tables {
        expr_ty: vec![Ty::Error; program.expr_count as usize],
        builtin_ranges: program.builtin_ranges.clone(),
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
        body::check_fn(program, FnId(f as u32), &mut t, &mut diags);
    }
    collect_ty::check_map_values(&t, &mut diags);
    let ok = !diags.iter().any(|d| d.is_error());
    (ok.then_some(t), diags)
}
