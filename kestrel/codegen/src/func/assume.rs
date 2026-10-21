//! `@assume` (`Stmt::Assume`): each condition in order, a false one stopping the program with its
//! text and line, then the body.

use cranelift_codegen::ir::{InstBuilder, TrapCode};
use cranelift_module::Module;
use kestrel_kir as kir;

use super::FuncLowerer;

impl<'a, 'b, M: Module> FuncLowerer<'a, 'b, M> {
    /// Lowers an assumption; returns whether control can reach its end. A trusted one's conditions
    /// are evaluated only when proofs are verified (`KESTREL_CHECK_PROOFS=1`), so a false one is
    /// reported as the program's error, not as a removed check failing.
    pub(super) fn assume(&mut self, conds: &[(u32, kir::Expr)], check: kir::AssumeCheck, body: &[kir::Stmt]) -> Result<bool, String> {
        if check == kir::AssumeCheck::Check || self.shared.opts.verify_proofs {
            for (id, cond) in conds {
                let info = self.shared.kir.assumptions.get(*id as usize).ok_or_else(|| format!("assumption {id} has no text"))?;
                let (text, line) = (info.text.clone(), info.line);
                let holds = self.expr(cond)?;
                let (fail, ok) = (self.b.create_block(), self.b.create_block());
                self.b.ins().brif(holds, ok, &[], fail, &[]);
                self.b.switch_to_block(fail);
                self.b.set_cold_block(fail);
                let (p, n) = self.string(&text)?;
                let (l, f) = self.loc_args(line);
                self.call_id(self.shared.rt.assume_failed, &[p, n, l, f]);
                self.b.ins().trap(TrapCode::unwrap_user(2));
                self.b.switch_to_block(ok);
            }
        }
        self.stmts(body)
    }
}
