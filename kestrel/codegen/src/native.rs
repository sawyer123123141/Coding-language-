//! `native` functions: bodyless Kestrel declarations whose code is a Rust
//! (or C) `extern "C"` function, imported under the symbol `Func::native`.

use cranelift_codegen::ir::{types, Signature, Type};
use cranelift_codegen::isa::CallConv;
use kestrel_kir as kir;

use crate::func;

/// The signature of a native import. It follows Kestrel's own convention
/// (see `func::signature`): scalars in registers, aggregates by pointer, a
/// struct result through a pointer passed first. The one addition is that
/// an `i8` argument (`bool`, `u8`) is zero-extended, because the C ABI
/// leaves its upper bits undefined unless the caller extends them.
pub(crate) fn signature(f: &kir::Func, cc: CallConv, ptr: Type) -> Signature {
    let mut sig = func::signature(f, cc, ptr);
    for p in &mut sig.params {
        if p.value_type == types::I8 {
            *p = p.uext();
        }
    }
    sig
}
