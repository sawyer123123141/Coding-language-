mod body;
mod collect;
pub mod tables;
pub mod ty;

use kestrel_syntax::ast::Program;
use kestrel_syntax::Diagnostic;

pub use tables::{ConstId, ConstInfo, FnId, FnInfo, LocalId, LocalInfo, ParamSig, Res, Tables};
pub use ty::Ty;

/// Resolves names and checks types. Tables are returned only when there
/// are no errors; warnings may still be present.
pub fn check(program: &Program) -> (Option<Tables>, Vec<Diagnostic>) {
    let mut t = Tables { expr_ty: vec![Ty::Error; program.expr_count as usize], ..Tables::default() };
    let mut diags = Vec::new();
    collect::collect(program, &mut t, &mut diags);
    body::check_consts(program, &mut t, &mut diags);
    for f in 0..t.fns.len() {
        body::check_fn(program, FnId(f as u32), &mut t, &mut diags);
    }
    let ok = !diags.iter().any(|d| d.is_error());
    (ok.then_some(t), diags)
}
