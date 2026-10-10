use kestrel_kir::*;
use kestrel_syntax::{parse, LineIndex};

fn lower_inlined(src: &str) -> Module {
    let (p, d) = parse(src);
    assert!(d.is_empty(), "parse: {:?}", d.iter().map(|d| &d.message).collect::<Vec<_>>());
    let (t, d) = kestrel_check::check(&p);
    assert!(d.is_empty(), "check: {:?}", d.iter().map(|d| &d.message).collect::<Vec<_>>());
    let mut m = lower(&p, &t.unwrap(), &LineIndex::new(src));
    inline::inline_small(&mut m);
    m
}

fn func<'m>(m: &'m Module, name: &str) -> &'m Func {
    m.funcs.iter().find(|f| f.name == name).unwrap()
}

fn calls(stmts: &[Stmt]) -> usize {
    let mut n = 0;
    visit::walk(stmts, &mut |node| {
        if matches!(node, visit::Node::Expr(Expr { kind: ExprKind::Call(..), .. }) | visit::Node::Stmt(Stmt::Call(..))) {
            n += 1;
        }
    });
    n
}

#[test]
fn small_functions_are_inlined_and_recursive_ones_are_not() {
    let src = "pure sq(x: int) -> int { x * x }\npure fib(n: int) -> int {\n if n < 2 { n } else { fib(n - 1) + fib(n - 2) }\n}\nf(a: int) -> int { sq(a) + sq(a + 1) + fib(a) }\nmain() {}\n";
    let m = lower_inlined(src);
    // Only the call to `fib` is left.
    assert_eq!(calls(&func(&m, "f").body), 1);
    assert_eq!(calls(&func(&m, "fib").body), 2);
}

#[test]
fn var_self_methods_inline_onto_the_receiver_place() {
    let src = "struct P {\n x: int\n bump(var self, by: int) {\n  self.x += by\n }\n}\nmain() {\n var ps = [P(1), P(2)]\n ps[1].bump(5)\n print(ps[1].x)\n}\n";
    let m = lower_inlined(src);
    let main = func(&m, "main");
    assert_eq!(calls(&main.body), 0, "{:#?}", main.body);
    // The receiver's index was evaluated once, into a temporary, and the body assigns through it.
    let assigns: Vec<&Place> = main.body.iter().filter_map(|s| if let Stmt::Assign(p, _) = s { Some(p) } else { None }).collect();
    assert!(matches!(assigns[..], [p] if matches!(p.path[..], [Proj::Index { .. }, Proj::Field(0)])), "{:#?}", main.body);
}

/// A pure function of `x` whose body is a sum of `terms` products: about
/// four KIR nodes per term.
fn sum_fn(name: &str, terms: usize) -> String {
    let body: Vec<String> = (1..=terms).map(|i| format!("x * {i}")).collect();
    format!("pure {name}(x: int) -> int {{ {} }}\n", body.join(" + "))
}

#[test]
fn mid_sized_functions_are_inlined() {
    // Like a noise octave: too big for the old 64-node limit.
    let src = format!("{}f(a: int) -> int {{ mid(a) + mid(a + 1) }}\nmain() {{}}\n", sum_fn("mid", 30));
    let m = lower_inlined(&src);
    assert_eq!(calls(&func(&m, "f").body), 0);
}

#[test]
fn calls_in_the_arguments_of_inlined_calls_are_inlined() {
    // Like `mix(lattice(..), lattice(..), t)` in a noise octave.
    let src = "pure sq(x: int) -> int { x * x }\npure add(a: int, b: int) -> int { a + b }\nf(a: int) -> int { add(sq(a), sq(sq(a + 1))) }\nmain() {}\n";
    let m = lower_inlined(src);
    assert_eq!(calls(&func(&m, "f").body), 0, "{:#?}", func(&m, "f").body);
}

#[test]
fn chains_deeper_than_four_calls_are_inlined() {
    // solids -> density -> octave -> lattice -> hash, inside a mesher.
    let src = "pure e(x: int) -> int { x + 1 }\npure d(x: int) -> int { e(x) * 2 }\npure c(x: int) -> int { d(x) + 3 }\npure b(x: int) -> int { c(x) - 4 }\npure a(x: int) -> int { b(x) * 5 }\nf(x: int) -> int { a(x) }\nmain() {}\n";
    let m = lower_inlined(src);
    assert_eq!(calls(&func(&m, "f").body), 0, "{:#?}", func(&m, "f").body);
}

#[test]
fn inlining_stops_when_the_caller_has_grown_enough() {
    let uses: Vec<String> = (0..60).map(|i| format!("mid(a + {i})")).collect();
    let src = format!("{}f(a: int) -> int {{ {} }}\nmain() {{}}\n", sum_fn("mid", 30), uses.join(" + "));
    let m = lower_inlined(&src);
    let left = calls(&func(&m, "f").body);
    assert!(left > 0 && left < 60, "{left} of 60 calls left");
}

#[test]
fn functions_with_early_returns_stay_calls() {
    let src = "pick(a: int) -> int {\n if a > 0 { return 1 }\n 2\n}\nf() -> int { pick(3) }\nmain() {}\n";
    let m = lower_inlined(src);
    assert_eq!(calls(&func(&m, "f").body), 1);
}
