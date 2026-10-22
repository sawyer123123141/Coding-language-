//! Puts every branch out of a loop first in its `brif`, so Cranelift hoists loop-invariant code.
//!
//! Cranelift 0.116's egraph elaborator walks the dominator tree, children in CFG reverse
//! postorder, keeping a stack of the loops it is in. When it reaches a block outside a loop it
//! pops that loop for good: a later child of the same loop block, back inside the loop, then
//! hoists nothing. Its depth-first search takes a `brif`'s first target first, which puts that
//! target *last* among its siblings, so a loop written `brif more, body, exit` elaborates the exit
//! first and never hoists out of the body (wasmtime PR #14383 fixed this upstream, in wasmtime 50).
//! Swapping such branches to `brif !more, exit, body` puts the body first.
//!
//! This covers the loop's own test, a `break` or `return` in an `if` and the checks (which already
//! branch to their failure first). It can't cover everything: a block after an `if` always comes
//! after the `if`'s branches, so a branch that holds a check or a `break` still stops hoisting in
//! the code after the `if` until Cranelift is upgraded.

use cranelift_codegen::cursor::{Cursor, FuncCursor};
use cranelift_codegen::dominator_tree::DominatorTree;
use cranelift_codegen::flowgraph::ControlFlowGraph;
use cranelift_codegen::ir::condcodes::{CondCode, IntCC};
use cranelift_codegen::ir::{Function, InstBuilder, InstructionData, Value, ValueDef};
use cranelift_codegen::loop_analysis::LoopAnalysis;

/// Swaps the targets of each `brif` in a loop whose first target stays in its innermost loop and
/// whose second leaves it, negating the condition.
pub(crate) fn put_loop_exits_first(func: &mut Function) {
    let cfg = ControlFlowGraph::with_function(func);
    let domtree = DominatorTree::with_function(func, &cfg);
    let mut loops = LoopAnalysis::new();
    loops.compute(func, &cfg, &domtree);
    let blocks: Vec<_> = func.layout.blocks().collect();
    for block in blocks {
        let (Some(lp), Some(inst)) = (loops.innermost_loop(block), func.layout.last_inst(block)) else { continue };
        let InstructionData::Brif { arg, blocks: [then_b, else_b], .. } = func.dfg.insts[inst] else { continue };
        let pool = &func.dfg.value_lists;
        let (then_b, else_b) = (then_b.block(pool), else_b.block(pool));
        if !loops.is_in_loop(then_b, lp) || loops.is_in_loop(else_b, lp) {
            continue;
        }
        let not = negate(func, inst, arg);
        if let InstructionData::Brif { arg, blocks, .. } = &mut func.dfg.insts[inst] {
            *arg = not;
            blocks.swap(0, 1);
        }
    }
}

/// A value, computed just before `before`, that is nonzero exactly when `cond` is zero. A
/// comparison is redone with the opposite condition (for floats, the exact opposite, NaN
/// included), so the branch can still use the flags.
fn negate(func: &mut Function, before: cranelift_codegen::ir::Inst, cond: Value) -> Value {
    let cond = func.dfg.resolve_aliases(cond);
    let def = match func.dfg.value_def(cond) {
        ValueDef::Result(inst, 0) => Some(func.dfg.insts[inst]),
        _ => None,
    };
    let mut pos = FuncCursor::new(func).at_inst(before);
    match def {
        Some(InstructionData::IntCompare { cond: cc, args, .. }) => pos.ins().icmp(cc.complement(), args[0], args[1]),
        Some(InstructionData::IntCompareImm { cond: cc, arg, imm, .. }) => pos.ins().icmp_imm(cc.complement(), arg, imm),
        Some(InstructionData::FloatCompare { cond: cc, args, .. }) => pos.ins().fcmp(cc.complement(), args[0], args[1]),
        _ => pos.ins().icmp_imm(IntCC::Equal, cond, 0),
    }
}
