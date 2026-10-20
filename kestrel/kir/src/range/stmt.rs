//! Statements. A simple statement goes through the spec's four steps ("One statement at a time"):
//! forget what its expressions may write; analyse each operand from that state; decide its checks;
//! then add the facts from checks that passed on every path, forget the writes again, and apply
//! the statement's own effect. So a fact never outlives the value it is about, and the analysis
//! never depends on the order codegen evaluates operands in.

use super::state::State;
use super::{writes, Cx, Frame};
use crate::ir::*;

impl Cx<'_> {
    /// The state after `body`, run from `st`. Statements after the point where no state reaches
    /// are unreachable: their checks stay.
    pub(super) fn stmts(&mut self, mut st: State, body: &[Stmt]) -> State {
        for (i, s) in body.iter().enumerate() {
            if st.is_dead() {
                self.unreachable(&body[i..]);
                return st;
            }
            st = self.stmt(st, s);
        }
        st
    }

    /// Statements no state reaches: in a deciding pass their checks are listed as kept, and
    /// nothing flows out of them (a `break` there joins no loop's exit).
    pub(super) fn unreachable(&mut self, rest: &[Stmt]) {
        if self.deciding && !self.gave_up && !rest.is_empty() {
            self.keep += 1;
            let top = self.t.top();
            let _ = self.stmts(top, rest);
            self.keep -= 1;
        }
    }

    fn stmt(&mut self, st: State, s: &Stmt) -> State {
        if !self.enter(true) {
            return State::dead();
        }
        let out = match s {
            Stmt::If { cond, then_body, else_body } => {
                let (t, f) = self.condition(st, cond);
                let a = self.stmts(t, then_body);
                let b = self.stmts(f, else_body);
                a.join(&b)
            }
            Stmt::While { cond, body } => self.while_loop(st, cond, body),
            Stmt::ForRange { var, start, end, body } => self.for_range(st, *var, (start, end), body),
            Stmt::Break | Stmt::Continue => {
                if self.keep == 0 {
                    if let Some(frame) = self.frames.last_mut() {
                        let slot = if matches!(s, Stmt::Break) { &mut frame.breaks } else { &mut frame.continues };
                        join_into(slot, st);
                    }
                }
                State::dead()
            }
            Stmt::Return(_) => {
                let _ = self.simple(st, s);
                State::dead()
            }
            Stmt::Trap | Stmt::Fail(_) => State::dead(),
            _ => self.simple(st, s),
        };
        self.leave();
        out
    }

    /// A simple statement, through the four steps.
    fn simple(&mut self, st: State, s: &Stmt) -> State {
        let mut st = st;
        // 1. Forget what the expressions may write.
        self.forget(&mut st, &writes::of_operands(s));
        // 2 and 3. Each operand from that state, deciding its checks.
        let mark = self.learned.len();
        let value = self.operands(&st, s);
        // 4. Facts from checks that passed, then the writes again, then the statement's effect.
        let learned: Vec<_> = self.learned.drain(mark..).collect();
        for (l, r) in learned {
            st.narrow(&self.t, l, r);
        }
        self.forget(&mut st, &writes::of_stmt(s));
        match (s, value) {
            (Stmt::Let(l, _), Some(v)) => st.set(&self.t, *l, v),
            (Stmt::Assign(p, _), Some(v)) if p.path.is_empty() => st.set(&self.t, p.root, v),
            _ => {}
        }
        st
    }

    /// Analyses a simple statement's operands from `st`, in the order codegen evaluates them; the
    /// range of the value a `let` or an assignment stores.
    fn operands(&mut self, st: &State, s: &Stmt) -> Option<super::interval::Interval> {
        match s {
            Stmt::Let(_, e) | Stmt::Return(Some(e)) | Stmt::RandSeed(e) => return Some(self.eval(st, e, true)),
            Stmt::Assign(p, e) => {
                let v = self.eval(st, e, true);
                self.place(st, p, true);
                return Some(v);
            }
            Stmt::ListAdd(p, e) | Stmt::SetLen(p, e) => {
                self.eval(st, e, true);
                self.place(st, p, true);
            }
            Stmt::List(_, p, xs) => {
                self.all(st, xs, true);
                self.place(st, p, true);
            }
            Stmt::MapSet(p, k, v, _) => {
                self.eval(st, k, true);
                self.eval(st, v, true);
                self.place(st, p, true);
            }
            Stmt::MapUnlock(p, saved) => {
                self.eval(st, saved, true);
                self.place(st, p, true);
            }
            Stmt::Call(_, args) => self.args(st, args, true),
            Stmt::CallIndirect { callee: p, args, .. } | Stmt::CallVirtual { recv: p, args, .. } => {
                self.place(st, p, true);
                self.args(st, args, true);
            }
            Stmt::ParFor { args, count, .. } => {
                self.eval(st, count, true);
                self.args(st, args, true);
            }
            Stmt::Print(parts) | Stmt::Write(parts) => self.print_args(st, parts, true),
            Stmt::BindEnv { env, .. } => {
                self.eval(st, env, true);
            }
            Stmt::Bind(_, p)
            | Stmt::CopyFnEnv(p)
            | Stmt::CopyIface(p)
            | Stmt::Dup(p)
            | Stmt::FreeArena(p)
            | Stmt::DropGpu(p)
            | Stmt::PushArena(Some(p)) => self.place(st, p, true),
            Stmt::If { .. }
            | Stmt::While { .. }
            | Stmt::ForRange { .. }
            | Stmt::Break
            | Stmt::Continue
            | Stmt::Return(None)
            | Stmt::BindGlobal(..)
            | Stmt::PushArena(None)
            | Stmt::PopArena { .. }
            | Stmt::SetLine(_)
            | Stmt::Trap
            | Stmt::Fail(_) => {}
        }
        None
    }

    /// The states where `cond` is true and where it is false, through the spec's condition rule:
    /// forget everything the condition may write, refine with its parts in their order, add the
    /// facts from its checks that passed, then forget its writes again, so no arm, loop body or
    /// loop exit sees a fact about a value the condition changed after the fact was made.
    pub(super) fn condition(&mut self, st: State, cond: &Expr) -> (State, State) {
        let mut st = st;
        let w = writes::of_expr(cond);
        self.forget(&mut st, &w);
        let mark = self.learned.len();
        let (mut t, mut f) = self.cond_parts(st, cond, true);
        let learned: Vec<_> = self.learned.drain(mark..).collect();
        for (l, r) in learned {
            t.narrow(&self.t, l, r);
            f.narrow(&self.t, l, r);
        }
        self.forget(&mut t, &w);
        self.forget(&mut f, &w);
        (t, f)
    }

    /// Runs a loop body from `st` with a fresh frame: the state at its end, and the states its
    /// `break`s and `continue`s carried out.
    pub(super) fn body(&mut self, st: State, body: &[Stmt]) -> (State, Frame) {
        self.frames.push(Frame::default());
        let end = self.stmts(st, body);
        let frame = self.frames.pop().expect("pushed above");
        (end, frame)
    }
}

/// Joins `st` into a frame's slot (empty means no path yet).
fn join_into(slot: &mut Option<State>, st: State) {
    *slot = Some(match slot.take() {
        Some(old) => old.join(&st),
        None => st,
    });
}
