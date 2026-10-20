//! How an emitted check behaves: as it always has, not at all (range analysis proved it can't
//! fail), or verified (proven, but kept to stop the program if the proof was wrong).

use cranelift_codegen::ir::{types, InstBuilder, TrapCode, Value};
use cranelift_module::Module;
use kestrel_runtime::PANIC_BOUNDS;

use super::FuncLowerer;

/// What a check does in the code being emitted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Check {
    /// Today's behaviour: the check is emitted when the build asks for it (overflow, shift and
    /// narrowing checks only with `checked_arith`; division and bounds checks always).
    Emit,
    /// Range analysis proved the check can't fail: nothing is emitted.
    Skip,
    /// Proven, but emitted anyway: its failure calls `kestrel_rt_proof_failed`.
    Verify,
}

impl Check {
    /// Whether an overflow, shift or narrowing check is emitted, given the build's setting.
    pub(crate) fn arith(self, checked_arith: bool) -> bool {
        match self {
            Check::Emit => checked_arith,
            Check::Skip => false,
            Check::Verify => true,
        }
    }
}

impl<'a, 'b, M: Module> FuncLowerer<'a, 'b, M> {
    /// The mode of the checks a proven expression or index holds.
    pub(crate) fn proven_check(&self) -> Check {
        if self.shared.opts.verify_proofs {
            Check::Verify
        } else {
            Check::Skip
        }
    }

    /// When `cond` is true, fails: with the runtime panic of `kind` for `Emit`, or with the
    /// compiler-bug message for `Verify`. `Skip` emits nothing.
    pub(super) fn fail_if(&mut self, cond: Value, kind: i32, line: u32, check: Check) {
        if check == Check::Skip {
            return;
        }
        let (fail, ok) = (self.b.create_block(), self.b.create_block());
        self.b.ins().brif(cond, fail, &[], ok, &[]);
        self.b.switch_to_block(fail);
        self.b.set_cold_block(fail);
        self.call_failure(check, kind, line, None);
        self.b.ins().trap(TrapCode::unwrap_user(2));
        self.b.switch_to_block(ok);
    }

    /// Emits the call that ends the program for a failed check: `kestrel_rt_panic` (or, for a
    /// bounds check, `kestrel_rt_panic_index` with the index and length) when the check is
    /// emitted as always; `kestrel_rt_proof_failed` when it is verifying a proof.
    pub(super) fn call_failure(&mut self, check: Check, kind: i32, line: u32, index: Option<(Value, Value)>) {
        let (l, f) = self.loc_args(line);
        if check == Check::Verify {
            let k = self.b.ins().iconst(types::I32, i64::from(if index.is_some() { PANIC_BOUNDS } else { kind }));
            self.call_id(self.shared.rt.proof_failed, &[k, l, f]);
        } else if let Some((i, len)) = index {
            self.call_id(self.shared.rt.panic_index, &[i, len, l, f]);
        } else {
            let k = self.b.ins().iconst(types::I32, i64::from(kind));
            self.call_id(self.shared.rt.panic, &[k, l, f]);
        }
    }
}
