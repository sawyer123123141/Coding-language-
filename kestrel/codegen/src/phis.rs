//! Removes the block parameters that only ever take one value, so a loop doesn't seem to change
//! a variable it never assigns.
//!
//! Lowering seals every block at the end (`seal_all_blocks`), so `cranelift_frontend` gives a
//! loop header a parameter for every variable read inside the loop, and decides whether to keep
//! it before it knows the value the back edge brings. An inner loop that reads an outer loop's
//! variable `y` keeps `block5(y5)` reached with `jump block5(y1)` and `jump block5(y5)`: always
//! `y1`, the outer header's parameter. Cranelift's own pass (`remove_constant_phis`) only removes
//! a parameter whose one value isn't itself a parameter, so `y5` stays, and everything computed
//! from `y` looks loop-variant to the elaborator and is never hoisted out of the inner loop.
//!
//! A parameter whose arguments are all one value `x` or the parameter itself is replaced by `x`
//! (in valid SSA `x` dominates the block), until no more go.

use cranelift_codegen::ir::{Block, Function, Inst, Value};
use cranelift_codegen::entity::SecondaryMap;

/// Replaces every block parameter that always takes the same value with that value.
pub(crate) fn remove_redundant_params(func: &mut Function) {
    let Some(entry) = func.layout.entry_block() else { return };
    // Each block's incoming edges: the branch and which of its destinations goes there.
    let mut edges: SecondaryMap<Block, Vec<(Inst, usize)>> = SecondaryMap::new();
    for block in func.layout.blocks() {
        let Some(inst) = func.layout.last_inst(block) else { continue };
        for (i, call) in func.dfg.insts[inst].branch_destination(&func.dfg.jump_tables, &func.dfg.exception_tables).iter().enumerate() {
            edges[call.block(&func.dfg.value_lists)].push((inst, i));
        }
    }
    let blocks: Vec<Block> = func.layout.blocks().filter(|&b| b != entry).collect();
    let mut changed = true;
    while changed {
        changed = false;
        for &block in &blocks {
            let mut ix = 0;
            while ix < func.dfg.num_block_params(block) {
                let param = func.dfg.block_params(block)[ix];
                match only_value(func, &edges[block], ix, param) {
                    Some(value) => {
                        for &(inst, i) in &edges[block] {
                            let dfg = &mut func.dfg;
                            dfg.insts[inst].branch_destination_mut(&mut dfg.jump_tables, &mut dfg.exception_tables)[i].remove(ix, &mut dfg.value_lists);
                        }
                        func.dfg.remove_block_param(param);
                        func.dfg.change_to_alias(param, value);
                        changed = true;
                    }
                    None => ix += 1,
                }
            }
        }
    }
}

/// The one value other than `param` that the edges pass as parameter `ix`, if there is one.
fn only_value(func: &Function, edges: &[(Inst, usize)], ix: usize, param: Value) -> Option<Value> {
    let mut only = None;
    for &(inst, i) in edges {
        let call = func.dfg.insts[inst].branch_destination(&func.dfg.jump_tables, &func.dfg.exception_tables)[i];
        // Kestrel makes no `try_call`, so every argument is a value.
        let v = func.dfg.resolve_aliases(call.args(&func.dfg.value_lists).nth(ix)?.as_value()?);
        if v == param {
            continue;
        }
        match only {
            None => only = Some(v),
            Some(o) if o == v => {}
            Some(_) => return None,
        }
    }
    only
}
