//! Closure literals passed straight to a call: which get a stack environment
//! (`Tables::stack_closures`), and the call exclusivity rule that keeps what they capture, and
//! the place a called function value is read from, unchanged while the call runs.

use kestrel_check::{check, Tables};
use kestrel_syntax::parse;

fn run(src: &str) -> (Option<Tables>, Vec<String>) {
    let (p, d) = parse(src);
    assert!(d.is_empty(), "parse errors: {:?}", d.iter().map(|d| &d.message).collect::<Vec<_>>());
    let (t, d) = check(&p);
    (t, d.into_iter().filter(|d| d.is_error()).map(|d| d.message).collect())
}

fn ok(src: &str) -> Tables {
    let (t, errs) = run(src);
    assert!(errs.is_empty(), "unexpected errors: {errs:?}");
    t.expect("tables")
}

fn errors(src: &str) -> Vec<String> {
    run(src).1
}

/// For each closure literal in source order: whether it has a stack environment.
fn stack(t: &Tables) -> Vec<bool> {
    let mut ids: Vec<_> = t.closures.keys().copied().collect();
    ids.sort();
    ids.iter().map(|id| t.stack_closures.contains(id)).collect()
}

const DECLS: &str = "struct S {
    n: int = 0

    with(self, f: (int) -> int) -> int { f(self.n) }
}

struct Box {
    f: (int) -> int
}

apply(f: (int) -> int) -> int { f(1) }
";

#[test]
fn only_a_literal_that_is_directly_a_call_argument_gets_a_stack_environment() {
    let src = format!(
        "{DECLS}main() {{
    let k = 2
    let c = true
    var fs: list[(int) -> int] = []
    let s = S()
    let g = (h: (int) -> int) -> int {{ h(0) }}
    print(apply((x) {{ x + k }}))
    print(s.with((x) {{ x + k }}))
    print(g((x) {{ x + k }}))
    print(apply(if c {{ (x: int) -> int {{ x + k }} }} else {{ (x: int) -> int {{ x }} }}))
    fs.add((x) {{ x + k }})
    let b = Box(f: (x) {{ x + k }})
    let f = (x: int) {{ x + k }}
    print(apply((x) {{ x }}))
}}
"
    );
    let t = ok(&src);
    // `g` itself is stored; the literals passed to `apply`, `with` and `g` are direct arguments;
    // the ones in the `if`, `add`, the constructor and the `let` are not. A literal that captures
    // nothing is recorded too (it has no environment either way).
    assert_eq!(stack(&t), vec![false, true, true, true, false, false, false, false, false, true]);
}

#[test]
fn a_literal_passed_straight_to_a_call_may_capture_a_whole_value_holding_a_function() {
    let src = "struct B {
    name: string
    cb: () -> string = () { \"\" }

    go(self) -> string { self.name }

    show(self) -> string { run(() { self.go() }) }
}
run(f: () -> string) -> string { f() }
main() { }
";
    ok(src);
    // A stored closure around it copies the value, so it is refused there, once.
    let stored = "struct B {
    name: string
    cb: () -> string = () { \"\" }

    go(self) -> string { self.name }

    keep(self) -> () -> string { () { run(() { self.go() }) } }
}
run(f: () -> string) -> string { f() }
main() { }
";
    assert_eq!(errors(stored), vec!["this closure would copy all of `self`, including its function fields; copy what it needs into a local first"]);
}

const CHANGED: &str = "is changed by this call, so the closure passed to it can't read it";

#[test]
fn a_call_may_change_what_the_literal_does_not_capture() {
    ok("struct Counter {
    count: int = 0
    cb: (var int) = (var n) { n += 1 }
}
struct H {
    items: list[int]
    label: string
}
tag(var xs: list[int], name: () -> string) { xs.add(name().len) }
main() {
    var b = Counter()
    b.cb(var b.count)
    var h = H(items: [1], label: \"h\")
    tag(var h.items, () { h.label })
}
");
}

#[test]
fn changes_inside_arguments_and_moves_conflict() {
    let decls = "struct H {
    mem: Arena = Arena()
    name: string = \"\"

    reset(var self, f: () -> string) { }
}
struct Box {
    h: H
}
later(f: () -> string, n: int) { }
size(b: Box) -> int { 1 }
bump(var n: int) -> int { n }
";
    // A `var self` receiver, a sibling argument's assignment, and a move into a construction.
    let src = format!(
        "{decls}main() {{
    var h = H()
    h.reset(() {{ h.name }})
    var n = 1
    later(() {{ h.name }}, if n > 0 {{ bump(var n) }} else {{ 0 }})
    later(() {{ \"{{n}}\" }}, if n > 0 {{
        n = 2
        1
    }} else {{
        0
    }})
    later(() {{ h.name }}, size(Box(h: h)))
}}
"
    );
    let msgs: Vec<String> = ["h", "n", "h"].iter().map(|n| format!("`{n}` {CHANGED}")).collect();
    // The first `later` changes `n`, which its literal doesn't capture; the second assigns it.
    let errs = errors(&src);
    assert_eq!(errs, vec![msgs[0].clone(), msgs[1].clone(), msgs[2].clone()], "{errs:?}");
}

#[test]
fn a_called_value_cannot_be_changed_by_the_call() {
    let decls = "struct W {
    mem: Arena = Arena()
    cb: (int) -> int = (x) { x }
}
renew(var w: W) -> int { 1 }
";
    // A change inside an argument counts too.
    let src = format!("{decls}main() {{\n    var w = W()\n    print(w.cb(renew(var w)))\n}}\n");
    assert_eq!(errors(&src), vec!["can't call `w.cb` while this call changes `w`, which may hold its memory"]);
    // A function value made inside `in w.mem { }` may live in `w.mem`.
    let src = format!(
        "{decls}main() {{\n    var w = W()\n    let k = 1\n    in w.mem {{\n        let g = (x: int) {{ x + k }}\n        print(g(renew(var w)))\n    }}\n}}\n"
    );
    assert_eq!(errors(&src), vec!["can't call `g` while this call changes `w`, which may hold its memory"]);
}
