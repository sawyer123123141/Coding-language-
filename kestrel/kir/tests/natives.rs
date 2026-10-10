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
