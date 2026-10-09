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

#[test]
fn functions_with_early_returns_stay_calls() {
    let src = "pick(a: int) -> int {\n if a > 0 { return 1 }\n 2\n}\nf() -> int { pick(3) }\nmain() {}\n";
    let m = lower_inlined(src);
    assert_eq!(calls(&func(&m, "f").body), 1);
}
