//! `@prove(deterministic)`: what breaks it (a native, the shared random generator, a function
//! value that isn't `pure`, an interface method that is neither `pure` nor promised), the summary
//! chain that follows it through calls, and the help lines. (The errors themselves:
//! `tests/errors/prove_deterministic_*.kes`.)

mod range_common;

use kestrel_kir::range::contracts::{check, ContractError};
use kestrel_kir::range::events::{events, Event};
use kestrel_kir::range::summary::{summarize, Summary};
use kestrel_kir::range::{remove_checks, Arith};
use kestrel_kir::*;
use range_common::{idx, line, optimized};

/// Every function's events.
fn all_events(m: &Module) -> Vec<Vec<Event>> {
    m.funcs.iter().map(|f| if f.native.is_some() { Vec::new() } else { events(m, f) }).collect()
}

/// `src` optimised and analysed (checked), and every function's summary.
fn summaries(src: &str) -> (Module, Vec<Summary>) {
    let mut m = optimized(src);
    let a = remove_checks(&mut m, Arith::Checked);
    let s = summarize(&m, &a, &all_events(&m), None);
    (m, s)
}

/// The contract errors of `src`, after `rename` has renamed functions of the optimised module (a
/// native declared here can't have a module's name, like `window.delta`).
fn errors_renamed(src: &str, rename: &[(&str, &str)]) -> Vec<ContractError> {
    let mut m = optimized(src);
    for (from, to) in rename {
        let i = idx(&m, from);
        m.funcs[i].name = to.to_string();
    }
    let a = remove_checks(&mut m, Arith::Checked);
    check(&m, &a)
}

fn errors(src: &str) -> Vec<ContractError> {
    errors_renamed(src, &[])
}

/// A function that stays a call: it has an early `return`.
const KEEP: &str = "if g.n < 0 {\n        return\n    }";

#[test]
fn effects_follow_calls_into_a_chain() {
    let src = format!(
        "struct Game {{\n    n: int\n}}
pub spawn_enemy(var g: Game) {{\n    {KEEP}\n    g.n = g.n ^ randi_range(1, 6)\n}}
@prove(deterministic)
pub step(var g: Game) {{\n    spawn_enemy(var g)\n}}"
    );
    let (m, s) = summaries(&src);
    let step = s[idx(&m, "step")].effects.as_ref().expect("`step` reaches the shared random generator");
    assert_eq!(step.steps.len(), 1, "{step:?}");
    assert_eq!(m.funcs[step.steps[0].callee.0 as usize].name, "spawn_enemy");
    assert_eq!(step.reason.via, "calls `randi_range`");
    assert_eq!(step.reason.tail, ", which uses the program's shared random generator");
    assert_eq!(line(step.reason.line), 8);
    let own = s[idx(&m, "spawn_enemy")].effects.as_ref().expect("`spawn_enemy` has it itself");
    assert!(own.steps.is_empty());
}

#[test]
fn print_and_math_keep_a_function_deterministic() {
    let src = "@prove(deterministic)
pub tell(var xs: list[float], y: float) -> float {
    print(\"y is {y}\")
    xs.sort()
    par for x in xs {
        x = x * 2.0
    }
    sin(y) + atan2(y, 1.0) + pow(y, 2.0) + sqrt(y)
}";
    let found: Vec<String> = errors(src).into_iter().map(|e| e.message).collect();
    assert_eq!(found, Vec::<String>::new());
}

#[test]
fn a_pure_function_always_holds() {
    let src = "interface Shape {\n    pure area(self) -> float\n}
@prove(deterministic)
pub pure through_value(f: pure (float) -> float, x: float) -> float { f(x) }
@prove(deterministic)
pub pure through_method(s: Shape) -> float { s.area() }
@prove(deterministic)
pub pure count(n: int) -> int {
    if n <= 0 {
        return 0
    }
    count(n - 1) + 1
}";
    let found: Vec<String> = errors(src).into_iter().map(|e| e.message).collect();
    assert_eq!(found, Vec::<String>::new());
}

#[test]
fn every_help_line_says_what_to_do() {
    // The natives are named as the built-in modules name theirs.
    let src = "native delta() -> float
native held(k: int) -> bool
struct Fighter {\n    hp: int\n}
@prove(deterministic)
pub clock() -> float { delta() }
@prove(deterministic)
pub random() -> float { randf() }
@prove(deterministic)
pub keys() -> bool { held(1) }
@prove(deterministic)
pub hit(var f: Fighter, on_hit: (Fighter) -> int) {\n    f.hp = f.hp ^ on_hit(f)\n}";
    let found: Vec<(String, Option<String>)> = errors_renamed(src, &[("delta", "window.delta"), ("held", "input.held")]).into_iter().map(|e| (e.message, e.help)).collect();
    let want = [
        ("`clock` can't be proven `deterministic`: it calls `window.delta` (line 7), which reads the clock", "pass the time step in as a parameter, like `step(var g, i, DT)`"),
        (
            "`random` can't be proven `deterministic`: it calls `randf` (line 9), which uses the program's shared random generator",
            "keep an `Rng` from the `random` module in the game state and call `g.rng.randi_range(1, 6)`",
        ),
        ("`keys` can't be proven `deterministic`: it calls `input.held` (line 11), which reads the keyboard", "read input outside and pass it in an `Inputs` struct"),
        ("`hit` can't be proven `deterministic`: it calls the function value `on_hit` (line 14), which isn't `pure`", "give it a `pure` function type, like `pure (Fighter) -> int`"),
    ];
    let want: Vec<(String, Option<String>)> = want.iter().map(|(m, h)| (m.to_string(), Some(h.to_string()))).collect();
    assert_eq!(found, want);
}

#[test]
fn the_mouse_window_and_method_help_lines_say_what_to_do() {
    let src = "native mouse() -> vec2
native clear()
interface Plain {\n    score(self) -> int\n}
@prove(deterministic)
pub aim() -> vec2 { mouse() }
@prove(deterministic)
pub wipe() { clear() }
@prove(deterministic)
pub c(s: Plain) -> int { s.score() }";
    let found: Vec<(String, Option<String>)> = errors_renamed(src, &[("mouse", "input.mouse"), ("clear", "draw.clear")]).into_iter().map(|e| (e.message, e.help)).collect();
    let want = [
        ("`aim` can't be proven `deterministic`: it calls `input.mouse` (line 7), which reads the mouse", "read input outside and pass it in an `Inputs` struct"),
        (
            "`wipe` can't be proven `deterministic`: it calls `draw.clear` (line 9), which talks to the window, which a deterministic function can't depend on",
            "talk to the window outside and pass what the function needs in as parameters",
        ),
        (
            "`c` can't be proven `deterministic`: it calls a method of the interface value `s` (line 11), which isn't `pure` and doesn't promise `deterministic`",
            "add `@prove(deterministic)` to the interface method `Plain.score`; then every implementation must keep it",
        ),
    ];
    let want: Vec<(String, Option<String>)> = want.iter().map(|(m, h)| (m.to_string(), Some(h.to_string()))).collect();
    assert_eq!(found, want);
}

#[test]
fn the_time_args_and_env_help_lines_say_what_to_do() {
    let src = "native now() -> f64
native sleep(s: f64)
native all() -> int
native get() -> int
@prove(deterministic)
pub t() -> f64 { now() }
@prove(deterministic)
pub nap() { sleep(1.0) }
@prove(deterministic)
pub words() -> int { all() }
@prove(deterministic)
pub var_of() -> int { get() }";
    let rename = [("now", "time.now"), ("sleep", "time.sleep"), ("all", "args.all"), ("get", "env.get")];
    let found: Vec<(String, Option<String>)> = errors_renamed(src, &rename).into_iter().map(|e| (e.message, e.help)).collect();
    let want = [
        ("`t` can't be proven `deterministic`: it calls `time.now` (line 6), which reads the clock", "pass the time step in as a parameter, like `step(var g, i, DT)`"),
        ("`nap` can't be proven `deterministic`: it calls `time.sleep` (line 8), which waits on the clock", "wait outside the function, after it returns"),
        (
            "`words` can't be proven `deterministic`: it calls `args.all` (line 10), which reads the command line",
            "read the command line outside and pass the value in as a parameter",
        ),
        (
            "`var_of` can't be proven `deterministic`: it calls `env.get` (line 12), which reads the environment",
            "read the environment outside and pass the value in as a parameter",
        ),
    ];
    let want: Vec<(String, Option<String>)> = want.iter().map(|(m, h)| (m.to_string(), Some(h.to_string()))).collect();
    assert_eq!(found, want);
}

#[test]
fn randf_range_is_named_as_written() {
    let src = "@prove(deterministic)\npub roll() -> float { randf_range(1.0, 2.0) }\n@prove(deterministic)\npub plain() -> float { randf() }";
    let found: Vec<String> = errors(src).into_iter().map(|e| e.message).collect();
    let want = [
        "`roll` can't be proven `deterministic`: it calls `randf_range` (line 2), which uses the program's shared random generator",
        "`plain` can't be proven `deterministic`: it calls `randf` (line 4), which uses the program's shared random generator",
    ];
    assert_eq!(found, want);
}

#[test]
fn randf_two_calls_deep_is_followed() {
    let src = "pub inner(n: int) -> float {\n    if n < 0 {\n        return 0.0\n    }\n    randf()\n}
pub outer(n: int) -> float {\n    if n < 0 {\n        return 0.0\n    }\n    inner(n)\n}
@prove(deterministic)
pub step(n: int) -> float { outer(n) }";
    let found: Vec<String> = errors(src).into_iter().map(|e| e.message).collect();
    let want = "`step` can't be proven `deterministic`: it calls `outer` (line 14), which calls `inner` (line 11), which calls `randf` (line 5), which uses the program's shared random generator";
    assert_eq!(found, [want]);
}

#[test]
fn a_helper_that_only_prints_keeps_it() {
    // `say` isn't `pure` (it prints) and stays a call (an early `return`).
    let src = "pub say(n: int) {\n    if n < 0 {\n        return\n    }\n    print(\"n is {n}\")\n}
@prove(deterministic)
pub step(n: int) -> int {\n    say(n)\n    n ^ 1\n}";
    let found: Vec<String> = errors(src).into_iter().map(|e| e.message).collect();
    assert_eq!(found, Vec::<String>::new());
}

#[test]
fn a_cut_walk_in_its_own_code_breaks_it() {
    // Only the walk is cut: the analysis found nothing wrong, so `deterministic` alone fails.
    let big = |f: Box<dyn FnOnce() + Send>| std::thread::Builder::new().stack_size(256 << 20).spawn(f).unwrap().join().unwrap();
    big(Box::new(|| {
        let mut m = optimized("@prove(deterministic)\npub f() {}");
        let truth = Expr { kind: ExprKind::Bool(true), ty: KTy::Bool, line: 0 };
        let mut body = Vec::new();
        for _ in 0..5000 {
            body = vec![Stmt::If { cond: truth.clone(), then_body: body, else_body: Vec::new() }];
        }
        let f = idx(&m, "f");
        m.funcs[f].body = body;
        let a = kestrel_kir::range::Analysis { funcs: vec![kestrel_kir::range::FuncChecks::default(); m.funcs.len()], assumptions: Vec::new() };
        let found: Vec<String> = check(&m, &a).into_iter().map(|e| e.message).collect();
        let want = "`f` can't be proven `deterministic`: range analysis stopped early in it (its code nests too deeply), so nothing can be proven about it; split it into smaller functions";
        assert_eq!(found, [want]);
    }));
}

#[test]
fn a_cut_walk_breaks_it_through_a_call() {
    // A walk cut short may hide an effect; range analysis giving up alone doesn't matter.
    let big = |f: Box<dyn FnOnce() + Send>| std::thread::Builder::new().stack_size(256 << 20).spawn(f).unwrap().join().unwrap();
    big(Box::new(|| {
        let mut m = optimized("pub g() {}\npub f() {\n    g()\n}");
        let truth = Expr { kind: ExprKind::Bool(true), ty: KTy::Bool, line: 0 };
        let mut body = Vec::new();
        for _ in 0..5000 {
            body = vec![Stmt::If { cond: truth.clone(), then_body: body, else_body: Vec::new() }];
        }
        let (g, f) = (idx(&m, "g"), idx(&m, "f"));
        m.funcs[g].body = body;
        m.funcs[f].body = vec![Stmt::Call(FuncIdx(g as u32), Vec::new(), pack_line(0, 3))];
        let a = kestrel_kir::range::Analysis { funcs: vec![kestrel_kir::range::FuncChecks::default(); m.funcs.len()], assumptions: Vec::new() };
        let s = summarize(&m, &a, &all_events(&m), None);
        let chain = s[f].effects.as_ref().expect("`f` reaches the cut");
        assert_eq!(chain.steps.len(), 1);
        assert_eq!(chain.reason.tail, ", where range analysis stopped early (its code nests too deeply)");
    }));
}

#[test]
fn the_other_contracts_ignore_effects() {
    // `randf` neither allocates, nor has a check, nor fails; the effect is for `deterministic` only.
    let src = "@prove(no_alloc, no_checks, no_panic)\npub roll() -> float { randf() }";
    let found: Vec<String> = errors(src).into_iter().map(|e| e.message).collect();
    assert_eq!(found, Vec::<String>::new());
}

const SNAPSHOT_HELP: &str = "keep arenas, textures, meshes, interface values and function values outside the state a deterministic function changes";

#[test]
fn the_snapshot_fault_has_help() {
    let src = "struct World {\n    mem: Arena\n    n: int\n}
@prove(deterministic)
pub step(var w: World) {\n    w.n += 1\n}";
    let found: Vec<(String, Option<String>)> = errors(src).into_iter().map(|e| (e.message, e.help)).collect();
    let message = "`step` can't be proven `deterministic`: its `var` parameter `w` holds an `Arena` (`w.mem`), which a snapshot can't copy and checksum";
    assert_eq!(found, vec![(message.to_string(), Some(SNAPSHOT_HELP.to_string()))]);
}

#[test]
fn the_snapshot_fault_is_said_before_what_the_code_does() {
    // Both break the contract; the error names the state, which the caller can fix without
    // reading the body.
    let src = "struct World {\n    mem: Arena\n    n: int\n}
@prove(deterministic)
pub step(var w: World) {\n    w.n += randi_range(1, 6)\n}";
    let found: Vec<String> = errors(src).into_iter().map(|e| e.message).collect();
    assert_eq!(found.len(), 1, "{found:?}");
    assert!(found[0].contains("its `var` parameter `w` holds an `Arena` (`w.mem`)"), "{found:?}");
}

#[test]
fn a_var_self_and_an_implementation_are_checked_too() {
    let src = "struct World {\n    mem: Arena\n    n: int\n
    @prove(deterministic)
    bump(var self) {\n        self.n += 1\n    }
}
interface Steps {
    @prove(deterministic)
    step(var self)
}
struct Sim: Steps {\n    keep: () -> int\n
    step(var self) {\n    }
}";
    let found: Vec<String> = errors(src).into_iter().map(|e| e.message).collect();
    let want = [
        "`World.bump` can't be proven `deterministic`: its `var` parameter `self` holds an `Arena` (`self.mem`), which a snapshot can't copy and checksum",
        "`Sim.step` can't be proven `deterministic`, which `Steps.step` requires: its `var` parameter `self` holds a function value (`self.keep`), which a snapshot can't copy and checksum",
    ];
    assert_eq!(found, want);
}

#[test]
fn a_read_only_parameter_and_plain_data_have_no_snapshot_error() {
    let src = "struct World {\n    mem: Arena\n    n: int\n}
struct Flat {\n    n: int\n    xs: list[int]\n}
@prove(deterministic)
pub look(w: World) -> int {\n    w.n\n}
@prove(deterministic)
pub bump(var f: Flat) {\n    f.n += 1\n}";
    let found: Vec<String> = errors(src).into_iter().map(|e| e.message).collect();
    assert_eq!(found, Vec::<String>::new());
}
