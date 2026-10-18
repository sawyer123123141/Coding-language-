//! Function signatures: the types of function values. A signature is interned like an element
//! type, so two function types are the same type exactly when their `SigId`s are equal.

use crate::tables::{FnId, Tables};
use crate::ty::Ty;

/// An interned signature, an index into `Tables::sigs`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SigId(pub u32);

/// One parameter of a signature. Function types have no parameter names.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SigParam {
    pub ty: Ty,
    /// Changed in place by the function, so the caller marks the argument `var`.
    pub is_var: bool,
}

/// What a function value takes and gives, and whether calling it is pure.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Sig {
    pub params: Vec<SigParam>,
    pub ret: Ty,
    pub is_pure: bool,
}

impl Tables {
    pub fn intern_sig(&mut self, sig: Sig) -> SigId {
        if let Some(&id) = self.sig_ids.get(&sig) {
            return id;
        }
        let id = SigId(self.sigs.len() as u32);
        self.sigs.push(sig.clone());
        self.sig_ids.insert(sig, id);
        id
    }

    pub fn sig(&self, id: SigId) -> &Sig {
        &self.sigs[id.0 as usize]
    }

    /// The signature of function `f`, which is the type of `f` used as a value.
    pub fn fn_sig(&self, f: FnId) -> SigId {
        self.func(f).sig
    }

    /// A function type as it reads in messages: `(int, var vec2) -> bool`, `pure (int) -> int`,
    /// and `(int)` for one that gives nothing.
    pub fn sig_name(&self, id: SigId) -> String {
        let sig = self.sig(id);
        let params: Vec<String> =
            sig.params.iter().map(|p| format!("{}{}", if p.is_var { "var " } else { "" }, self.ty_name(p.ty))).collect();
        let ret = if sig.ret == Ty::Unit { String::new() } else { format!(" -> {}", self.ty_name(sig.ret)) };
        format!("{}({}){ret}", if sig.is_pure { "pure " } else { "" }, params.join(", "))
    }
}

/// Whether a function value of signature `got` may go where `want` is expected: the same
/// parameters (and which are `var`) and the same result, and a pure function goes where a
/// non-pure one is expected, never the other way round.
pub fn sig_accepts(t: &Tables, want: SigId, got: SigId) -> bool {
    if want == got {
        return true;
    }
    let (w, g) = (t.sig(want), t.sig(got));
    w.params == g.params && w.ret == g.ret && (g.is_pure || !w.is_pure)
}
