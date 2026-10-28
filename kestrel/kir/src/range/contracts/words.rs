//! The words of a contract's error: what it is on, which copy, the reason (the first item that
//! breaks it, in the report's words), and the help line.

use kestrel_check::SnapshotFault;
use kestrel_syntax::ast::Contract;

use super::region::{Copy, Lost};
use crate::ir::*;
use crate::range::events::Effect;
use crate::range::report::{callee_name, display_name};
use crate::range::report_copies::plain_name;
use crate::range::sites::CheckKind;
use crate::range::summary::{chain_rest, chain_text, check_words, is_builtin_body, line_ref, reason_text, Chain, Reason, Stop};

/// What a site is on: "`step`", "`Goblin.area`" or "the loop at line 12". A copy of a generic adds
/// where it came from: "`Stack[string].put` (made by `Stack[string]` on line 12)", "`best[int]`
/// (made on line 12)" for a copy that made itself, "the loop at line 5 (made by `Box[string]`)"
/// for a loop in an instance's field default (nothing in the code names a line for it).
pub(super) fn target(m: &Module, site: &ContractSite) -> String {
    let (named, func, home) = match site.target {
        ProveTarget::Func(f) => (format!("`{}`", display_name(m, f)), f, m.funcs[f.0 as usize].lines.0),
        ProveTarget::Loop { func, line } => (format!("the loop at line {}", unpack_line(line).0), func, line),
    };
    // A loop's code may have moved to a closure's: the site says which copy it was lowered for.
    let copy = match &site.copy {
        Some(SiteCopy::Instance(name)) => return format!("{named} (made by `{}`)", plain_name(m, name, home)),
        Some(SiteCopy::Func(c)) => *c,
        None => func,
    };
    let Some(note) = m.copies.get(&copy) else { return named };
    let at = on(m, note.made_at, unpack_line(home).1);
    let itself = matches!(site.target, ProveTarget::Func(f) if m.funcs[f.0 as usize].name == note.made_by);
    if itself {
        return format!("{named} (made {at})");
    }
    format!("{named} (made by `{}` {at})", plain_name(m, &note.made_by, home))
}

/// Which copy an error is about, after the contract: nothing for the site's own, otherwise
/// " where it is inlined into `step` (line 40)" (a loop's copy names the function only).
pub(super) fn copy_where(m: &Module, site: &ContractSite, copy: Copy, home: u32) -> String {
    let own = match site.target {
        ProveTarget::Func(f) => copy.func == f.0 as usize,
        ProveTarget::Loop { func, .. } => copy.func == func.0 as usize,
    };
    if own {
        return String::new();
    }
    let into = callee_name(m, FuncIdx(copy.func as u32));
    match (site.target, copy.block) {
        (ProveTarget::Func(_), Some(b)) if b.at != 0 => format!(" where it is inlined into {into} ({})", line_ref(m, b.at, home)),
        _ => format!(" where it is inlined into {into}"),
    }
}

/// The whole message of a site the compiler lost track of.
pub(super) fn lost(l: &Lost) -> String {
    match l {
        Lost::Func => "`@prove` lost track of the function it is on; this is a compiler bug".to_string(),
        Lost::Loop(line) => format!("`@prove` lost track of the loop at line {}; this is a compiler bug", unpack_line(*line).0),
    }
}

/// "on line 14", or "on geom.kes:14" in another file than `home`.
fn on(m: &Module, line: u32, home: u32) -> String {
    format!("on {}", line_ref(m, line, home))
}

/// "it has a bounds check on line 14: {text}".
pub(super) fn check(m: &Module, kind: CheckKind, line: u32, text: &str, home: u32) -> String {
    format!("it has {} check {}: {text}", check_words(kind), on(m, line, home))
}

/// "it allocates on line 13: `add` may grow `ys`" (the item is at `line`).
pub(super) fn allocates(m: &Module, r: &Reason, line: u32, home: u32) -> String {
    format!("it allocates {}: {}", on(m, line, home), reason_text(m, r, home))
}

/// "it can fail on line 9: the assumption `n <= xs.len` is checked here" (the item is at `line`).
pub(super) fn fails(m: &Module, r: &Reason, line: u32, home: u32) -> String {
    format!("it can fail {}: {}", on(m, line, home), reason_text(m, r, home))
}

/// An unknown call in the region: "it calls the function value `f` (line 8); its body isn't known
/// here".
pub(super) fn unknown(m: &Module, r: &Reason, home: u32) -> String {
    format!("it {}", reason_text(m, r, home))
}

/// An unknown call for `inline`, made at `line` when the reason has none: "it calls the function
/// value `f` (line 8), which can't be inlined".
pub(super) fn not_inlinable(m: &Module, lead: &str, r: &Reason, line: u32, home: u32) -> String {
    let at = if r.line != 0 { r.line } else { line };
    format!("{lead} {} ({}), which can't be inlined", r.via, line_ref(m, at, home))
}

/// A call the inliner left: "it calls `big` (line 7), which isn't inlined: it has an early
/// `return`".
pub(super) fn not_inlined(m: &Module, lead: &str, callee: FuncIdx, line: u32, why: &str, home: u32) -> String {
    format!("{lead} calls {} ({}), which isn't inlined: {why}", callee_name(m, callee), line_ref(m, line, home))
}

/// "it can call itself (line 9), so the stack may run out".
pub(super) fn calls_itself(m: &Module, line: u32, home: u32) -> String {
    format!("it can call itself ({}), so the stack may run out", line_ref(m, line, home))
}

/// A call at `line` to `callee`, through whose chain the contract breaks: "it " and the chain's
/// words. A built-in's own code with the reason in it reads as an item of the copy ("it allocates
/// on line 5: `filter` builds a new list"), with `verb` ("allocates", "has a bounds check").
pub(super) fn call(m: &Module, callee: FuncIdx, chain: &Chain, line: u32, verb: &str) -> String {
    if chain.steps.is_empty() && is_builtin_body(m, callee) {
        if let Some(direct) = &chain.reason.direct {
            let at = if chain.reason.line != 0 { chain.reason.line } else { line };
            return format!("it {verb} {}: {direct}", on(m, at, unpack_line(line).1));
        }
    }
    format!("it {}", chain_text(m, callee, chain, line))
}

/// A call through an interface (in `r`'s words) to an implementation whose chain is `chain`: "it
/// calls a method of the interface value `s` (line 8), which can call itself (line 12), so the
/// stack may run out".
pub(super) fn through_method(m: &Module, r: &Reason, chain: &Chain, line: u32) -> String {
    let at = if r.line != 0 { r.line } else { line };
    format!("it {} ({}){}", r.via, line_ref(m, at, unpack_line(line).1), chain_rest(m, chain, at))
}

/// The constants' initializer, through whose chain a contract on `main` breaks: "it builds the
/// program's constants first, which makes a new list (line 1)".
pub(super) fn constants(m: &Module, chain: &Chain, line: u32) -> String {
    let r = &chain.reason;
    // (The constants' own code has no lines: a reason in it is said without one.)
    if chain.steps.is_empty() && r.line == 0 && !r.via.is_empty() {
        return format!("{CONSTANTS}, which {}{}", r.via, r.tail);
    }
    format!("{CONSTANTS}{}", chain_rest(m, chain, line))
}

/// How the reason for `main`'s constants starts.
pub(super) const CONSTANTS: &str = "it builds the program's constants first";

/// Range analysis couldn't decide function `g` (`it` when that is the site's own function).
pub(super) fn stopped(m: &Module, stop: Stop, g: usize, it: bool) -> String {
    let name = if it { "it".to_string() } else { callee_name(m, FuncIdx(g as u32)) };
    let (did, why) = stop.words();
    match stop {
        Stop::Undecided => format!("range analysis {did} in {name}{why}, so nothing can be proven about it"),
        _ => format!("range analysis {did} in {name}{why}, so nothing can be proven about it; split it into smaller functions"),
    }
}

/// A `var` parameter a snapshot can't copy: "its `var` parameter `w` holds an `Arena` (`w.mem`),
/// which a snapshot can't copy and checksum".
pub(super) fn snapshot(fault: &SnapshotFault) -> String {
    format!("its `var` parameter `{}` holds {} (`{}`), which a snapshot can't copy and checksum", fault.param, fault.kind.holds(), fault.path)
}

/// The help line of an error.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Help {
    /// The report lists everything left.
    Report,
    /// A call through a function value.
    FnValue,
    /// A call through an interface whose method (the function) doesn't promise the contract.
    Iface(FuncIdx),
    /// A `par for`'s arenas.
    ParFor,
    /// What breaks `deterministic`.
    Effect(Effect),
    /// A `var` parameter a snapshot can't copy.
    Snapshot,
}

/// The words of `help` for an error about contract `c`.
pub(super) fn help(m: &Module, help: Help, c: Contract) -> String {
    match help {
        Help::Effect(effect) => match effect {
            Effect::Clock => "pass the time step in as a parameter, like `step(var g, i, DT)`".to_string(),
            Effect::Keyboard | Effect::Mouse => "read input outside and pass it in an `Inputs` struct".to_string(),
            Effect::Window => "talk to the window outside and pass what the function needs in as parameters".to_string(),
            Effect::SharedRandom => "keep an `Rng` from the `random` module in the game state and call `g.rng.randi_range(1, 6)`".to_string(),
            Effect::ImpureFnValue => "give it a `pure` function type, like `pure (Fighter) -> int`".to_string(),
            Effect::ImpureMethod(method) => self::help(m, Help::Iface(method), c),
        },
        Help::Snapshot => "keep arenas, textures, meshes, interface values and function values outside the state a deterministic function changes".to_string(),
        Help::Report => "`kestrel check --report` lists everything left in it".to_string(),
        Help::FnValue => "contracts can't see into a function value yet; call a named function or an interface method instead".to_string(),
        Help::Iface(method) => {
            let name = &m.funcs[method.0 as usize].name;
            format!("add `@prove({})` to the interface method `{name}`; then every implementation must keep it", c.name())
        }
        Help::ParFor => "a `par for` makes an arena for each thread; put `@prove(no_alloc)` on the function its body calls".to_string(),
    }
}
