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

#[test]
fn a_native_lowers_to_a_bodyless_import() {
    let m = lower_inlined("pub native twice(x: int, v: vec2) -> int\nnative origin() -> vec2\nkeep() {}\nmain() {}\n");
    let twice = func(&m, "twice");
    assert_eq!(twice.native.as_deref(), Some("kestrel_twice"));
    assert!(twice.body.is_empty());
    assert_eq!((twice.params.len(), twice.ret), (2, Some(KTy::I32)));
    assert_eq!(twice.locals[1].kind, LocalKind::RefParam, "a vec2 argument is passed by pointer");
    assert!(!twice.exported, "a native is never exported, even when `pub`");
    assert!(twice.io && !twice.is_pure);
    assert!(matches!(func(&m, "origin").ret, Some(KTy::Struct(_))));
    assert_eq!(func(&m, "keep").native, None);
    assert_eq!(func(&m, "main").native, None);
}

#[test]
fn calls_to_natives_stay_calls() {
    // A native has an empty body, which the inliner would otherwise splice in as nothing.
    let m = lower_inlined("native bump()\nnative twice(x: int) -> int\nmain() {\n bump()\n print(twice(2))\n}\n");
    let mut calls = 0;
    visit::walk(&func(&m, "main").body, &mut |node| {
        calls += matches!(node, visit::Node::Expr(Expr { kind: ExprKind::Call(..), .. }) | visit::Node::Stmt(Stmt::Call(..))) as u32;
    });
    assert_eq!(calls, 2);
}

const NATIVES: &str = "native len(s: string) -> int\nnative bump(s: string)\nnative twice(x: int) -> int\nnative both(s: string, k: int)\nshout(s: string) -> int { return 1 }\n";

/// The arena statements in `name`, in walk order: `push`, `pop` (freeing) or `keep` (not freeing).
fn arena_ops(m: &Module, name: &str) -> Vec<&'static str> {
    let mut ops = Vec::new();
    visit::walk(&func(m, name).body, &mut |node| {
        if let visit::Node::Stmt(s) = node {
            match s {
                Stmt::PushArena(None) => ops.push("push"),
                Stmt::PopArena { free: true } => ops.push("pop"),
                Stmt::PushArena(Some(_)) | Stmt::PopArena { free: false } => ops.push("keep"),
                _ => {}
            }
        }
    });
    ops
}

#[test]
fn an_interpolated_argument_is_built_in_a_scratch_arena_freed_after_the_call() {
    let m = lower_inlined(&format!(
        "{NATIVES}main() {{\n let n = 3\n let k = len(\"x{{n}}\")\n bump(\"y{{n}}\")\n print(k)\n}}\n"
    ));
    let body = &func(&m, "main").body;
    // A value-returning call: the arena is pushed, the call's result kept in a temporary,
    // the arena popped, and only then is the result read.
    let Stmt::Let(_, Expr { kind: ExprKind::Seq(inside, result), .. }) = &body[1] else { panic!("{body:#?}") };
    assert!(matches!(inside.first(), Some(Stmt::PushArena(None))), "{inside:#?}");
    assert!(matches!(inside.last(), Some(Stmt::PopArena { free: true })), "{inside:#?}");
    assert!(matches!(&inside[1], Stmt::Let(_, Expr { kind: ExprKind::Call(..), .. })), "{inside:#?}");
    assert_eq!(inside.len(), 3, "{inside:#?}");
    assert!(matches!(result.kind, ExprKind::Load(_)), "{result:#?}");
    // A call with no result: the same three statements, in the caller's own list.
    assert!(matches!(body[2], Stmt::PushArena(None)), "{body:#?}");
    assert!(matches!(&body[3], Stmt::Call(..)), "{body:#?}");
    assert!(matches!(body[4], Stmt::PopArena { free: true }), "{body:#?}");
    assert_eq!(body.len(), 6, "{body:#?}");
}

#[test]
fn calls_without_an_interpolated_argument_get_no_scratch_arena() {
    let m = lower_inlined(&format!(
        "{NATIVES}main() {{
 let n = 3
 let name = \"x{{n}}\"
 let a = len(\"plain\")
 let b = len(name)
 let c = twice(n)
 bump(name)
 bump(\"also plain\")
 let d = shout(\"x{{n}}\")
 print(a, b, c, d)
}}
"
    ));
    assert_eq!(arena_ops(&m, "main"), Vec::<&str>::new());
}

#[test]
fn only_the_native_call_holding_the_text_gets_the_arena() {
    // `twice(...)` takes no text, so only the `len` inside it gets an arena. In `both`, the
    // text is one argument and another native call holds more: the inner arena is popped
    // inside the outer one.
    let m = lower_inlined(&format!(
        "{NATIVES}main() {{
 let n = 3
 let a = twice(len(\"x{{n}}\"))
 both(\"a{{n}}\", len(\"b{{n}}\"))
 print(a)
}}
"
    ));
    assert_eq!(arena_ops(&m, "main"), ["push", "pop", "push", "push", "pop", "pop"]);
}

/// Every statement list in `stmts`: its own, and those inside loops, `if`s and expressions.
fn lists(stmts: &[Stmt]) -> Vec<&[Stmt]> {
    let mut out = vec![stmts];
    // `walk` reaches nested nodes itself, so each node adds only its own lists.
    visit::walk(stmts, &mut |node| match node {
        visit::Node::Stmt(Stmt::If { then_body, else_body, .. }) | visit::Node::Expr(Expr { kind: ExprKind::If { then_body, else_body, .. }, .. }) => {
            out.extend([&then_body[..], &else_body[..]]);
        }
        visit::Node::Stmt(Stmt::While { body, .. } | Stmt::ForRange { body, .. }) | visit::Node::Expr(Expr { kind: ExprKind::Seq(body, _), .. }) => {
            out.push(body);
        }
        _ => {}
    });
    out
}

/// How many `return`, `break` and `continue` statements in `name` come right after a
/// scratch-arena pop, and how many there are in all.
fn exits_after_a_pop(m: &Module, name: &str) -> (usize, usize) {
    let (mut popped, mut total) = (0, 0);
    for list in lists(&func(m, name).body) {
        for (i, s) in list.iter().enumerate() {
            if matches!(s, Stmt::Return(_) | Stmt::Break | Stmt::Continue) {
                total += 1;
                popped += (i > 0 && matches!(list[i - 1], Stmt::PopArena { free: true })) as usize;
            }
        }
    }
    (popped, total)
}

#[test]
fn leaving_early_from_inside_an_argument_pops_the_scratch_arena() {
    let m = lower_inlined(&format!(
        "{NATIVES}early(c: bool) -> int {{
 return len(\"a{{if c {{ return 5 }} else {{ 2 }}}}\")
}}
loops() {{
 var i = 0
 while i < 5 {{
  i += 1
  let a = len(\"b{{if i == 2 {{ continue }} else {{ i }}}}\")
  bump(\"c{{if i == 3 {{ break }} else {{ i }}}}\")
 }}
}}
main() {{}}
"
    ));
    // The `return` in the argument leaves through the pop; the final `return` comes after
    // the call's own pop, so there is nothing more to pop there.
    assert_eq!(exits_after_a_pop(&m, "early"), (1, 2));
    assert_eq!(arena_ops(&m, "early"), ["push", "pop", "pop"]);
    // `continue` and `break` each leave their call's arena, in both call forms.
    assert_eq!(exits_after_a_pop(&m, "loops"), (2, 2));
    assert_eq!(arena_ops(&m, "loops"), ["push", "pop", "pop", "push", "pop", "pop"]);
}

/// Lowers `src`, where the source from the line `// built-in` on is a built-in module's.
fn lower_with_built_ins(src: &str, inline: bool) -> Module {
    let (mut p, d) = parse(src);
    assert!(d.is_empty(), "parse: {:?}", d.iter().map(|d| &d.message).collect::<Vec<_>>());
    p.builtin_ranges = std::iter::once(src.find("// built-in").expect("a marker line") as u32..src.len() as u32).collect();
    let (t, d) = kestrel_check::check(&p);
    assert!(d.is_empty(), "check: {:?}", d.iter().map(|d| &d.message).collect::<Vec<_>>());
    let mut m = lower(&p, &t.unwrap(), &LineIndex::new(src));
    if inline {
        inline::inline_small(&mut m);
    }
    m
}

/// What `name` does that matters to the line a native's error names, in running order: the
/// `SetLine`s and the calls.
#[derive(Debug, PartialEq)]
enum Step {
    Line(u32),
    Call(String),
}

fn steps(m: &Module, name: &str) -> Vec<Step> {
    let mut steps = Vec::new();
    visit::walk(&func(m, name).body, &mut |node| match node {
        visit::Node::Stmt(Stmt::SetLine(n)) => steps.push(Step::Line(*n)),
        visit::Node::Expr(Expr { kind: ExprKind::Call(f, _), .. }) | visit::Node::Stmt(Stmt::Call(f, _)) => {
            steps.push(Step::Call(m.funcs[f.0 as usize].name.clone()));
        }
        _ => {}
    });
    steps
}

fn call(name: &str) -> Step {
    Step::Call(name.to_string())
}

#[test]
fn calls_into_built_in_modules_record_their_line() {
    let src = "main() {
 print(held(2))
 print(axis())
 print(mixf(1))
}
// built-in
native held(k: int) -> bool
pure native mixf(x: float) -> float
axis() -> float { if held(1) { 1.0 } else { 0.0 } }
";
    // A call to a built-in function that isn't `pure` records its line, whether the function is
    // a native or written in Kestrel. A `pure` one doesn't, and neither do the calls that
    // built-in code makes.
    for inline in [false, true] {
        let m = lower_with_built_ins(src, inline);
        let main = steps(&m, "main");
        let lines: Vec<_> = main.iter().filter(|s| matches!(s, Step::Line(_))).collect();
        assert_eq!(lines, [&Step::Line(2), &Step::Line(3)], "inline {inline}: {main:?}");
        // Each line is set right before its call.
        assert_eq!(main[..2], [Step::Line(2), call("held")], "inline {inline}: {main:?}");
        let at = main.iter().position(|s| *s == Step::Line(3)).unwrap();
        assert!(main[at + 1..].iter().all(|s| !matches!(s, Step::Line(_))), "inline {inline}: {main:?}");
        assert_eq!(steps(&m, "axis"), [call("held")], "inline {inline}");
    }
}

const PAINTING: &str = "main() {
 paint(1,
  size())
 paint(pos(), 2)
}
pos() -> int { size() + 1 }
// built-in
native size() -> int
native paint(a: int, b: int)
";

#[test]
fn a_call_in_the_arguments_does_not_take_over_the_line() {
    // `size()` runs while the arguments are evaluated, and sets its own line. This call's line
    // is stored after every argument, so it is the one still set when `paint` runs.
    let m = lower_with_built_ins(PAINTING, false);
    let main = steps(&m, "main");
    assert_eq!(main[..4], [Step::Line(3), call("size"), Step::Line(2), call("paint")], "{main:?}");
}

#[test]
fn a_user_function_in_the_arguments_does_not_take_over_the_line() {
    // `pos` calls `size`, which sets a line inside `pos` (line 6). Its call is evaluated before
    // this call's line is stored, whether or not `pos` is inlined afterwards.
    for inline in [false, true] {
        let m = lower_with_built_ins(PAINTING, inline);
        assert_eq!(steps(&m, "pos"), [Step::Line(6), call("size")], "inline {inline}");
        let main = steps(&m, "main");
        assert_eq!(main[main.len() - 2..], [Step::Line(4), call("paint")], "inline {inline}: {main:?}");
    }
}
