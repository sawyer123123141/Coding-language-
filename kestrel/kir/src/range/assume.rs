//! `@assume` (`Stmt::Assume`). For now an assumption teaches the analysis nothing: its conditions
//! are analysed as an expression statement's operands are (their own checks decided, the facts
//! from the checks that passed kept), and its body is analysed as a plain block.

use super::state::State;
use super::{writes, Cx};
use crate::ir::*;

impl Cx<'_> {
    /// The state after evaluating an assumption's conditions, in order, from `st`.
    pub(super) fn assume_conds(&mut self, st: State, conds: &[(u32, Expr)]) -> State {
        let mut st = st;
        for (_, cond) in conds {
            let w = writes::of_expr(cond);
            self.forget(&mut st, &w);
            let mark = self.learned.len();
            self.within(&w, |cx| {
                cx.eval(&st, cond, true);
            });
            self.apply_learned(&mut st, mark);
            self.forget(&mut st, &w);
        }
        st
    }
}
