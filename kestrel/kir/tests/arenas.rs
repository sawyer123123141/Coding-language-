use kestrel_kir::*;
use kestrel_syntax::{parse, LineIndex};

fn lower_src(src: &str) -> Module {
    let src = format!("struct Chunk {{\n x: int\n mem: Arena = Arena()\n}}\n{src}");
    let (p, d) = parse(&src);
    assert!(d.is_empty(), "parse: {:?}", d.iter().map(|d| &d.message).collect::<Vec<_>>());
    let (t, d) = kestrel_check::check(&p);
    assert!(d.is_empty(), "check: {:?}", d.iter().map(|d| &d.message).collect::<Vec<_>>());
    lower(&p, &t.unwrap(), &LineIndex::new(&src))
}

fn func<'m>(m: &'m Module, name: &str) -> &'m Func {
    m.funcs.iter().find(|f| f.name == name).unwrap()
}

#[test]
fn break_out_of_an_arena_block_frees_it_first() {
    let m = lower_src("main() {\n while true {\n  arena {\n   let t = [1]\n   break\n  }\n }\n}\n");
    let Stmt::While { body, .. } = &func(&m, "main").body[0] else { panic!() };
    assert!(matches!(body[0], Stmt::PushArena(None, _)));
    let brk = body.iter().position(|s| matches!(s, Stmt::Break)).unwrap();
    assert!(matches!(body[brk - 1], Stmt::PopArena { free: true }), "{body:#?}");
}

#[test]
fn owning_locals_are_dropped_and_moves_zero_the_source() {
    let m = lower_src("main() {\n let a = Chunk(x: 1)\n let b = a\n}\n");
    let main = func(&m, "main");
    assert!(matches!(&main.body[1], Stmt::Let(_, Expr { kind: ExprKind::Move(_), .. })), "{:#?}", main.body);
    // Both are dropped at the end, newest first; `a` was zeroed by the move, so its drop does nothing.
    let frees: Vec<_> = main.body.iter().filter(|s| matches!(s, Stmt::FreeArena(_))).collect();
    assert_eq!(frees.len(), 2);
}

#[test]
fn lists_of_owning_values_drop_each_element() {
    let m = lower_src("main() {\n var w = list[Chunk]()\n w.add(Chunk(x: 1))\n}\n");
    assert!(m.funcs.iter().any(|f| f.name.starts_with("__kestrel_drop_")));
    let main = func(&m, "main");
    assert!(matches!(main.body.last(), Some(Stmt::Call(..))));
}

#[test]
fn return_inside_in_pops_before_returning() {
    let m = lower_src("f(var c: Chunk) -> int {\n in c.mem {\n  return 1\n }\n}\nmain() {}\n");
    let f = func(&m, "f");
    assert!(matches!(f.body[0], Stmt::PushArena(Some(_), _)));
    let ret = f.body.iter().position(|s| matches!(s, Stmt::Return(_))).unwrap();
    assert!(matches!(f.body[ret - 1], Stmt::PopArena { free: false }), "{:#?}", f.body);
}

#[test]
fn an_owning_value_that_is_never_stored_is_dropped_at_the_end_of_its_block() {
    let m = lower_src(
        "make() -> Chunk {\n Chunk(x: 1)\n}\nwidth(c: Chunk) -> int {\n c.x\n}\nmain() {\n make()\n print(width(make()), make().x)\n}\n",
    );
    let main = func(&m, "main");
    // Three temporaries (the discarded call, the argument, the base of `.x`) are each freed once, at the end.
    let frees = main.body.iter().filter(|s| matches!(s, Stmt::FreeArena(_))).count();
    assert_eq!(frees, 3, "{:#?}", main.body);
    assert!(matches!(main.body.last(), Some(Stmt::FreeArena(_))));
    // `width` only reads its parameter and `make` returns a fresh value: neither makes a temporary.
    assert!(func(&m, "width").body.iter().all(|s| !matches!(s, Stmt::FreeArena(_))));
    assert!(func(&m, "make").body.iter().all(|s| !matches!(s, Stmt::FreeArena(_))));
}
