//! `sort` and `sort_by`: `ListOp::Sort` with a generated compare function (`sort`, and `sort_by`
//! with a string key) or `ListOp::SortBy` with a generated key function (a number or bool key), a
//! call site that passes a literal key as a stack environment, and a scratch arena around the keys
//! only when they can build memory.

use kestrel_kir::*;
use kestrel_syntax::{parse, LineIndex};

fn lower_src(src: &str) -> Module {
    let (p, d) = parse(src);
    assert!(d.is_empty(), "parse: {:?}", d.iter().map(|d| &d.message).collect::<Vec<_>>());
    let (t, d) = kestrel_check::check(&p);
    assert!(d.is_empty(), "check: {:?}", d.iter().map(|d| &d.message).collect::<Vec<_>>());
    lower(&p, &t.unwrap(), &LineIndex::new(src))
}

fn named<'m>(m: &'m Module, prefix: &str) -> Vec<&'m Func> {
    m.funcs.iter().filter(|f| f.name.starts_with(prefix)).collect()
}

fn func<'m>(m: &'m Module, name: &str) -> &'m Func {
    m.funcs.iter().find(|f| f.name == name).unwrap_or_else(|| panic!("no function {name}"))
}

/// The top-level statements of `f` that open or close an arena.
fn arenas(f: &Func) -> Vec<&Stmt> {
    f.body.iter().filter(|s| matches!(s, Stmt::PushArena(..) | Stmt::PopArena { .. })).collect()
}

fn sorts(f: &Func) -> Vec<usize> {
    let mut out = Vec::new();
    visit::walk(&f.body, &mut |n| {
        if let visit::Node::Stmt(Stmt::List(ListOp::Sort | ListOp::SortBy(_), _, args)) = n {
            out.push(args.len());
        }
    });
    out
}

const PRELUDE: &str = "struct E {
    name: string
    hp: int
}

pure shout(e: E) -> int { e.name.upper().len }

";

#[test]
fn sort_is_one_runtime_sort_with_a_compare_function_per_item_type() {
    let m = lower_src("main() {
    var xs: list[int] = [3, 1]
    var ys: list[int] = [2, 1]
    var ws: list[string] = [\"b\", \"a\"]
    xs.sort()
    ys.sort()
    ws.sort()
}
");
    // Two item types, so two compare functions; the two int lists share one.
    let compares = named(&m, "__kestrel_compare_");
    assert_eq!(compares.len(), 2);
    assert_eq!(sorts(func(&m, "main")), vec![1, 1, 1]);
    for c in compares {
        // (env, a, b) -> u8, a and b by pointer; nothing to free.
        assert_eq!(c.params.len(), 3);
        assert_eq!(c.ret, Some(KTy::U8));
        assert_eq!([c.locals[1].kind, c.locals[2].kind], [LocalKind::RefParam; 2]);
        assert!(arenas(c).is_empty());
        assert!(c.is_pure);
    }
    assert_eq!(check_stack_envs(&m), Ok(()));
}

#[test]
fn a_literal_key_is_a_stack_environment_passed_straight_to_the_sorting_function() {
    let m = lower_src(&format!(
        "{PRELUDE}main() {{
    var es: list[E] = []
    let k = 2
    es.sort_by((e) {{ e.hp + k }})
}}
"
    ));
    assert_eq!(check_stack_envs(&m), Ok(()));
    let calls = named(&m, "__kestrel_sort_by_");
    assert_eq!(calls.len(), 1);
    assert_eq!(sorts(calls[0]), vec![2], "the key function, then the key to pass on by address");
    // `main` calls it with the list by reference and the literal itself.
    let mut found = false;
    visit::walk(&func(&m, "main").body, &mut |n| {
        if let visit::Node::Stmt(Stmt::Call(f, args, _)) = n {
            if m.funcs[f.0 as usize].name.starts_with("__kestrel_sort_by_") {
                found = true;
                assert!(matches!(&args[0], Arg::Ref(_)));
                assert!(matches!(&args[1], Arg::Value(Expr { kind: ExprKind::MakeFn { env: EnvKind::Stack(_), .. }, .. })));
            }
        }
    });
    assert!(found);
}

#[test]
fn a_key_that_cant_build_memory_gets_no_scratch_arena() {
    let m = lower_src(&format!(
        "{PRELUDE}main() {{
    var es: list[E] = []
    es.sort_by((e) {{ e.hp * 2 }})
    es.sort_by((e) {{ e.name.len }})
    es.sort_by(hp)
}}

pure hp(e: E) -> int {{ e.hp }}
"
    ));
    // (A named function's thunk is the code, so it is judged like a literal.)
    let keys = named(&m, "__kestrel_sort_key_");
    assert_eq!(keys.len(), 3);
    for c in keys {
        assert!(arenas(c).is_empty(), "{c:?}");
    }
}

#[test]
fn a_key_that_builds_strings_is_run_in_a_scratch_arena_freed_before_the_answer() {
    let m = lower_src(&format!(
        "{PRELUDE}main() {{
    var es: list[E] = []
    es.sort_by((e) {{ e.name.lower() }})
    es.sort_by((e) {{ shout(e) }})
    es.sort_by(shout)
    let by = (e: E) -> int {{ e.hp }}
    es.sort_by(by)
}}
"
    ));
    // The first builds a string itself (a string key: a compare function), the second calls a
    // function that does, the third is a named function that does, and the fourth is a stored
    // closure (anything may be inside it, so it keeps the arena even though this one builds
    // nothing); those three have number keys, so key functions.
    let compares = named(&m, "__kestrel_compare_");
    assert_eq!(compares.len(), 1);
    let keys = named(&m, "__kestrel_sort_key_");
    assert_eq!(keys.len(), 3);
    for c in compares.into_iter().chain(keys) {
        let a = arenas(c);
        assert_eq!(a.len(), 2, "{c:?}");
        assert!(matches!(a[0], Stmt::PushArena(None, _)));
        assert!(matches!(a[1], Stmt::PopArena { free: true }));
        // The arena is open first and closed just before the answer is returned.
        assert!(matches!(c.body.first(), Some(Stmt::PushArena(None, _))));
        assert!(matches!(&c.body[c.body.len() - 2], Stmt::PopArena { free: true }));
        assert!(matches!(c.body.last(), Some(Stmt::Return(Some(_)))));
    }
}

#[test]
fn a_function_value_key_is_called_indirectly_and_a_literal_directly() {
    let m = lower_src(&format!(
        "{PRELUDE}main() {{
    var es: list[E] = []
    let by = (e: E) -> int {{ e.hp }}
    es.sort_by((e) {{ e.hp }})
    es.sort_by(by)
}}
"
    ));
    let mut counts = Vec::new();
    for c in named(&m, "__kestrel_sort_key_") {
        let (mut direct, mut indirect) = (0, 0);
        visit::walk(&c.body, &mut |n| match n {
            visit::Node::Expr(Expr { kind: ExprKind::Call(..), .. }) => direct += 1,
            visit::Node::Expr(Expr { kind: ExprKind::CallIndirect { .. }, .. }) => indirect += 1,
            _ => {}
        });
        counts.push((direct, indirect));
    }
    counts.sort();
    // One call each: the key function keys one item.
    assert_eq!(counts, vec![(0, 1), (1, 0)]);
}

/// The kinds of sort in the `__kestrel_sort_by_*` functions of `m`, in order of the functions.
fn sort_ops(m: &Module) -> Vec<ListOp> {
    let mut out = Vec::new();
    for f in named(m, "__kestrel_sort_by_") {
        visit::walk(&f.body, &mut |n| {
            if let visit::Node::Stmt(Stmt::List(op, _, _)) = n {
                out.push(*op);
            }
        });
    }
    out
}

#[test]
fn a_number_or_bool_key_is_a_key_function_and_a_string_key_a_compare_function() {
    let m = lower_src(&format!(
        "{PRELUDE}main() {{
    var es: list[E] = []
    es.sort_by((e) {{ e.hp }})
    es.sort_by((e) {{ i64(e.hp) }})
    es.sort_by((e) {{ u8(e.hp & 7) }})
    es.sort_by((e) {{ e.hp > 3 }})
    es.sort_by((e) {{ float(e.hp) }})
    es.sort_by((e) {{ f64(e.hp) }})
    es.sort_by((e) {{ e.name }})
}}
"
    ));
    use SortKey::{Float, Int};
    let expected = [ListOp::SortBy(Int), ListOp::SortBy(Int), ListOp::SortBy(Int), ListOp::SortBy(Int), ListOp::SortBy(Float), ListOp::SortBy(Float), ListOp::Sort];
    assert_eq!(sort_ops(&m), expected);
    let keys = named(&m, "__kestrel_sort_key_");
    assert_eq!(keys.len(), 6);
    for k in &keys {
        // (env, a) -> i64 or f64, `a` by pointer, pure.
        assert_eq!(k.params.len(), 2);
        assert_eq!(k.locals[1].kind, LocalKind::RefParam);
        assert!(k.is_pure);
        let Some(Stmt::Return(Some(v))) = k.body.last() else { panic!("{k:?}") };
        assert_eq!(Some(v.ty), k.ret);
        let wide = if k.locals[2].ty.is_float() { KTy::F64 } else { KTy::I64 };
        assert_eq!(k.ret, Some(wide), "{k:?}");
        // A bool key is 0 or 1; a narrower number is converted; an `i64` or `f64` is as it is.
        match k.locals[2].ty {
            KTy::Bool => assert!(matches!(v.kind, ExprKind::If { .. }), "{v:?}"),
            KTy::I64 | KTy::F64 => assert!(matches!(v.kind, ExprKind::Load(_)), "{v:?}"),
            _ => assert!(matches!(v.kind, ExprKind::Cast(_)), "{v:?}"),
        }
    }
    assert_eq!(named(&m, "__kestrel_compare_").len(), 1);
}

#[test]
fn an_array_sorts_like_a_list() {
    let m = lower_src("main() {
    var a = array[int](3)
    a.sort()
    a.sort_by((n) { -n })
}
");
    assert_eq!(sorts(func(&m, "main")).len() + named(&m, "__kestrel_sort_by_").iter().map(|f| sorts(f).len()).sum::<usize>(), 2);
    assert_eq!(check_stack_envs(&m), Ok(()));
}
