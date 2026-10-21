//! `@prove` contracts as KIR keeps them: one `ContractSite` per function, method or loop that
//! promises something (an interface method's promise becomes a site on each implementing method),
//! listed in `Module::contracts`. The code a site covers is marked where it is not a whole
//! function: lowering wraps an annotated loop's statements in `Stmt::Prove { id }`, and the inliner
//! wraps each expanded copy of a contracted callee the same way (`id` is the site's index).
//! Range analysis tags each check and event with the `Prove` blocks around it (its regions).

use kestrel_syntax::ast::Contract;
use kestrel_syntax::Span;

use crate::ir::FuncIdx;

/// A set of contracts, one bit each.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct ContractSet(pub u8);

impl ContractSet {
    pub const NO_ALLOC: ContractSet = ContractSet(1);
    pub const NO_CHECKS: ContractSet = ContractSet(2);
    pub const NO_PANIC: ContractSet = ContractSet(4);
    pub const INLINE: ContractSet = ContractSet(8);

    /// The set holding just `c`'s bit (`no_panic` brings `no_checks`, as `insert` does).
    pub fn of(c: Contract) -> ContractSet {
        let mut set = ContractSet::default();
        set.insert(match c {
            Contract::NoAlloc => ContractSet::NO_ALLOC,
            Contract::NoChecks => ContractSet::NO_CHECKS,
            Contract::NoPanic => ContractSet::NO_PANIC,
            Contract::Inline => ContractSet::INLINE,
        });
        set
    }

    /// Whether every contract in `other` is in the set.
    pub fn contains(self, other: ContractSet) -> bool {
        self.0 & other.0 == other.0
    }

    /// Adds the contracts in `other`; `no_panic` includes `no_checks`, so it adds that too.
    pub fn insert(&mut self, other: ContractSet) {
        self.0 |= other.0;
        if self.contains(ContractSet::NO_PANIC) {
            self.0 |= ContractSet::NO_CHECKS.0;
        }
    }
}

/// What a site's contracts are about.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ProveTarget {
    /// A function or method: its own body, and every copy the inliner made of it.
    Func(FuncIdx),
    /// A `for`, `par for` or `while` loop: every `Stmt::Prove` block with the site's id. `func` is
    /// the function that holds the loop's own code (a `par for` body or a closure's code when the
    /// loop is written in one); `line` is the packed line of the loop's keyword.
    Loop { func: FuncIdx, line: u32 },
}

/// One function, method or loop and the contracts it must keep.
#[derive(Clone, Debug, PartialEq)]
pub struct ContractSite {
    pub target: ProveTarget,
    /// Each contract once, in the order written, with where it is written: inside `@prove(...)`,
    /// or, for a site made from an interface method, the implementing method's name.
    pub contracts: Vec<(Contract, Span)>,
    /// For a site made from an interface method's `@prove`: that method, as "`Shape.area`".
    pub required_by: Option<String>,
}
