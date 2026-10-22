//! `@assume` lowered: the rest of its block goes inside `Stmt::Assume`, a value block computes its
//! value inside it, and a function with one is still inlined.
use kestrel_kir::*;
use kestrel_syntax::{parse, LineIndex};

/// `src` (which needs no `main`) lowered.
fn lower_src(src: &str) -> Module {
    let src = format!("{src}\nmain() {{}}\n");
    let (p, d) = parse(&src);
    assert!(d.is_empty(), "parse: {:?}", d.iter().map(|d| &d.message).collect::<Vec<_>>());
    let (t, d) = kestrel_check::check(&p);
    assert!(d.is_empty(), "check: {:?}", d.iter().map(|d| &d.message).collect::<Vec<_>>());
    lower(&p, &t.unwrap(), &LineIndex::new(&src))
}

fn func<'m>(m: &'m Module, name: &str) -> &'m Func {
    m.funcs.iter().find(|f| f.name == name).unwrap_or_else(|| panic!("no function `{name}`"))
}

#[test]
fn the_rest_of_the_block_goes_inside() {
    let m = lower_src("f(n: int) {\n let a = n + 1\n @assume(n > 0)\n print(a)\n print(n)\n}");
    let body = &func(&m, "f").body;
    match &body[..] {
        [Stmt::Let(..), Stmt::Assume { conds, check: AssumeCheck::Check, body }] => {
            assert_eq!(conds.len(), 1);
            assert!(matches!(&body[..], [Stmt::Print(_), Stmt::Print(_)]), "{body:#?}");
        }
        other => panic!("{other:#?}"),
    }
}

#[test]
fn consecutive_assumptions_are_one() {
    for src in ["f(a: int, b: int) {\n @assume(a > 0) @assume(b > 0)\n print(a + b)\n}", "f(a: int, b: int) {\n @assume(a > 0)\n @assume(b > 0)\n print(a + b)\n}"] {
        let m = lower_src(src);
        match &func(&m, "f").body[..] {
            [Stmt::Assume { conds, body, .. }] => {
                let ids: Vec<u32> = conds.iter().map(|(id, _)| *id).collect();
                assert_eq!(ids, [0, 1]);
                assert!(matches!(&body[..], [Stmt::Print(_)]), "{body:#?}");
            }
            other => panic!("{other:#?}"),
        }
        let texts: Vec<&str> = m.assumptions.iter().map(|a| a.text.as_str()).collect();
        assert_eq!(texts, ["a > 0", "b > 0"]);
    }
}

#[test]
fn a_statement_between_assumptions_nests_them() {
    let m = lower_src("f(a: int, b: int) {\n @assume(a > 0)\n print(a)\n @assume(b > 0)\n print(b)\n}");
    match &func(&m, "f").body[..] {
        [Stmt::Assume { conds, body, .. }] => {
            assert_eq!(conds.len(), 1);
            assert!(matches!(&body[..], [Stmt::Print(_), Stmt::Assume { body: inner, .. }] if matches!(&inner[..], [Stmt::Print(_)])), "{body:#?}");
        }
        other => panic!("{other:#?}"),
    }
}

/// The body ends `[Assume { body: [.., Let(t, _)] }, Return(Some(Load(t) or Move(t)))]`.
fn ends_with_a_temporary(body: &[Stmt]) {
    let [.., Stmt::Assume { body: inner, .. }, Stmt::Return(Some(v))] = body else { panic!("{body:#?}") };
    let Some(Stmt::Let(t, _)) = inner.last() else { panic!("{inner:#?}") };
    match &v.kind {
        ExprKind::Load(p) | ExprKind::Move(p) => assert_eq!(*p, Place::local(*t)),
        other => panic!("{other:#?}"),
    }
}

const TAIL: &str = "pure g(n: int) -> int {\n @assume(n > 0)\n n * 2\n}";
const RETURN: &str = "pure g(n: int) -> int {\n @assume(n > 0)\n return n * 2\n}";

#[test]
fn a_tail_value_goes_through_a_temporary() {
    ends_with_a_temporary(&func(&lower_src(TAIL), "g").body);
}

#[test]
fn a_final_return_goes_after_the_assumption() {
    let m = lower_src(RETURN);
    let body = &func(&m, "g").body;
    ends_with_a_temporary(body);
    // The only `return` is the last statement.
    let mut returns = 0;
    visit::walk(body, &mut |n| returns += usize::from(matches!(n, visit::Node::Stmt(Stmt::Return(_)))));
    assert_eq!(returns, 1, "{body:#?}");
}

#[test]
fn an_if_arm_value_goes_through_a_temporary() {
    let m = lower_src("f(s: int) -> int {\n let v = if s > 0 {\n @assume(s > 0)\n s + 1\n } else {\n 0\n }\n v\n}");
    let body = &func(&m, "f").body;
    let Stmt::Let(_, Expr { kind: ExprKind::If { then_body, then_val: Some(v), .. }, .. }) = &body[0] else { panic!("{body:#?}") };
    let [Stmt::Assume { body: inner, .. }] = &then_body[..] else { panic!("{then_body:#?}") };
    let Some(Stmt::Let(t, _)) = inner.last() else { panic!("{inner:#?}") };
    assert!(matches!(&v.kind, ExprKind::Load(p) if *p == Place::local(*t)), "{v:#?}");
}

#[test]
fn a_returned_parameter_list_is_still_copied() {
    let m = lower_src("pure keep(xs: list[int]) -> list[int] {\n @assume(xs.len > 0)\n xs\n}");
    let body = &func(&m, "keep").body;
    let [Stmt::Assume { body: inner, .. }, Stmt::Return(Some(Expr { kind: ExprKind::Move(_), .. }))] = &body[..] else { panic!("{body:#?}") };
    // The value is a copy of the parameter (through the list's copy function), made inside the
    // assumption.
    let mut copies = 0;
    visit::walk(inner, &mut |n| copies += usize::from(matches!(n, visit::Node::Stmt(Stmt::Dup(_) | Stmt::Call(..)))));
    assert!(copies > 0, "{inner:#?}");
}

#[test]
fn a_function_with_an_assumption_is_still_inlined() {
    for g in [TAIL, RETURN] {
        let mut m = lower_src(&format!("{g}\nh(n: int) -> int {{ g(n) + 1 }}"));
        optimize(&mut m);
        let mut calls = 0;
        visit::walk(&func(&m, "h").body, &mut |n| {
            calls += usize::from(matches!(n, visit::Node::Expr(Expr { kind: ExprKind::Call(..), .. }) | visit::Node::Stmt(Stmt::Call(..))));
        });
        assert_eq!(calls, 0, "{:#?}", func(&m, "h").body);
        let g = FuncIdx(m.funcs.iter().position(|f| f.name == "g").unwrap() as u32);
        assert!(m.not_inlined.iter().all(|n| n.callee != g), "{:?}", m.not_inlined);
    }
}

#[test]
fn assumptions_keep_their_text() {
    let m = lower_src(TAIL.replace("\n @assume", " @assume").as_str());
    assert_eq!(m.assumptions[0].text, "n > 0");
    assert_eq!(unpack_line(m.assumptions[0].line), (1, 0));
}

/// `make` with `@assume(n >= 0)` after its first statement, ending in `ending` (`out` or
/// `return out`).
fn make_list(ending: &str, arena: bool) -> String {
    let arena = if arena { " var a = Arena()\n" } else { "" };
    format!("pure make(n: int) -> [u8] {{\n var out = array[u8](100)\n @assume(n >= 0)\n{arena} out[n & 63] = 1\n {ending}\n}}")
}

#[test]
fn a_returned_local_is_read_after_the_assumption_as_it_is() {
    // No temporary (one would hide the list's length from a caller's range analysis): the block
    // ends as it would without the assumption.
    for ending in ["out", "return out"] {
        let m = lower_src(&make_list(ending, false));
        let f = func(&m, "make");
        let [.., Stmt::Assume { .. }, Stmt::Return(Some(v))] = &f.body[..] else { panic!("{:#?}", f.body) };
        assert!(matches!(&v.kind, ExprKind::Load(p) if f.locals[p.root.0 as usize].name == "out" && p.path.is_empty()), "{ending}: {v:#?}");
    }
}

#[test]
fn with_a_cleanup_it_still_goes_through_a_temporary() {
    // The arena is declared inside the assumption and dropped after it: the value is settled first.
    for ending in ["out", "return out"] {
        let m = lower_src(&make_list(ending, true));
        let body = &func(&m, "make").body;
        let i = body.iter().position(|s| matches!(s, Stmt::Assume { .. })).expect("the assumption");
        let (Stmt::Assume { body: inner, .. }, Some(Stmt::Return(Some(v)))) = (&body[i], body.last()) else { panic!("{body:#?}") };
        assert!(body.len() > i + 2, "the arena's drop comes between: {body:#?}");
        let Some(Stmt::Let(t, _)) = inner.last() else { panic!("{inner:#?}") };
        assert!(matches!(&v.kind, ExprKind::Move(p) if *p == Place::local(*t)), "{ending}: {v:#?}");
    }
}
