//! Functions that stand for an `FnInfo` but are never run, so that `FuncIdx == FnId` holds for
//! every `FnInfo`: an interface's abstract method, and a generic function or method itself (its
//! copies are what code calls, G11).

use kestrel_check::{FnInfo, Ty};

use super::kty;
use crate::ir::*;

/// The function standing for an interface's method. Nothing calls it (calls go to the
/// implementing type's own method); its parameters follow the signature and its body stops the
/// program, so a stray call fails loudly.
pub(super) fn abstract_stub(info: &FnInfo) -> Func {
    let mut locals = Vec::new();
    for p in &info.params {
        let ty = kty(p.ty);
        let kind = if p.is_var || ty.is_aggregate() { LocalKind::RefParam } else { LocalKind::Value };
        locals.push(Local { name: p.name.clone(), ty, kind });
    }
    let params = (0..locals.len()).map(|i| LocalIdx(i as u32)).collect();
    let ret = if info.ret == Ty::Unit { None } else { Some(kty(info.ret)) };
    trap(info, params, ret, locals)
}

/// The function standing for a generic function or method (`generic` set, not a copy). Its
/// types hold placeholders, which have no runtime representation, so it has no parameters or
/// result: only its copies are lowered from its body.
pub(super) fn generic_stub(info: &FnInfo) -> Func {
    trap(info, Vec::new(), None, Vec::new())
}

fn trap(info: &FnInfo, params: Vec<LocalIdx>, ret: Option<KTy>, locals: Vec<Local>) -> Func {
    Func {
        name: info.name.clone(),
        params,
        ret,
        locals,
        body: vec![Stmt::Trap],
        is_pure: false,
        io: false,
        mutates_params: false,
        exported: false,
        lines: (0, 0),
        native: None,
    }
}
