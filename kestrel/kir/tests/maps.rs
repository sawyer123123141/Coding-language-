use kestrel_kir::*;
use kestrel_syntax::{parse, LineIndex};

fn lower_src(src: &str) -> Module {
    let (p, d) = parse(src);
    assert!(d.is_empty(), "parse: {:?}", d.iter().map(|d| &d.message).collect::<Vec<_>>());
    let (t, d) = kestrel_check::check(&p);
    assert!(d.is_empty(), "check: {:?}", d.iter().map(|d| &d.message).collect::<Vec<_>>());
    lower(&p, &t.unwrap(), &LineIndex::new(src))
}

fn func<'m>(m: &'m Module, name: &str) -> &'m Func {
    m.funcs.iter().find(|f| f.name == name).unwrap()
}

/// Every statement in `stmts`, nested ones included, outermost first.
fn all_stmts(stmts: &[Stmt]) -> Vec<&Stmt> {
    let mut out = Vec::new();
    visit::walk(stmts, &mut |n| {
        if let visit::Node::Stmt(s) = n {
            out.push(s);
        }
    });
    out
}

fn map_ops(stmts: &[Stmt]) -> Vec<MapOp> {
    let mut out = Vec::new();
    visit::walk(stmts, &mut |n| {
        if let visit::Node::Expr(Expr { kind: ExprKind::Map(op, ..), .. }) = n {
            out.push(*op);
        }
    });
    out
}

#[test]
fn layouts_put_values_after_keys() {
    let m = lower_src("main() {\n var a: map[int, i64] = {}\n var b: map[string, u8] = {}\n var c: map[u8, vec3] = {}\n}\n");
    let main = func(&m, "main");
    let tys: Vec<KTy> = main.body.iter().filter_map(|s| if let Stmt::Let(_, e) = s { Some(e.ty) } else { None }).collect();
    let layout = |ty| match ty {
        KTy::Map(k, v) => m.map_layout(k, v),
        other => panic!("{other:?}"),
    };
    assert_eq!(layout(tys[0]), MapLayout { key_kind: 1, val_off: 16, entry_size: 24, entry_align: 8 });
    assert_eq!(layout(tys[1]), MapLayout { key_kind: 3, val_off: 24, entry_size: 32, entry_align: 8 });
    assert_eq!(layout(tys[2]), MapLayout { key_kind: 0, val_off: 12, entry_size: 24, entry_align: 8 });
    assert_eq!(m.size_align(tys[0]), (64, 8));
}

#[test]
fn literals_sets_and_reads_lower_to_map_operations() {
    let m = lower_src(
        "main() {\n var m = {\"a\": 1, \"b\": 2}\n m[\"c\"] = 3\n let x = m[\"a\"]\n let y = m.get(\"z\", 0)\n let h = m.has(\"b\")\n m.remove(\"b\")\n print(x, y, h, m.len)\n}\n",
    );
    let main = func(&m, "main");
    let sets = all_stmts(&main.body).into_iter().filter(|s| matches!(s, Stmt::MapSet(..))).count();
    assert_eq!(sets, 3, "{:#?}", main.body);
    assert_eq!(map_ops(&main.body), [MapOp::Get, MapOp::Get, MapOp::Has, MapOp::Remove]);
}

#[test]
fn a_loop_over_a_map_locks_it_and_unlocks_it_on_every_way_out() {
    let m = lower_src(
        "f(var m: map[int, int]) -> int {\n for k, v in m {\n if v > 3 { return k }\n if v > 2 { break }\n }\n 0\n}\nmain() {}\n",
    );
    let f = func(&m, "f");
    let all = all_stmts(&f.body);
    let unlocks = all.iter().filter(|s| matches!(s, Stmt::MapUnlock(..))).count();
    // After the loop (normal end and `break`), and before the `return`.
    assert_eq!(unlocks, 2, "{:#?}", f.body);
    assert_eq!(map_ops(&f.body)[0], MapOp::Lock);
    let lock = f.body.iter().position(|s| matches!(s, Stmt::Let(_, Expr { kind: ExprKind::Map(MapOp::Lock, ..), .. })));
    let lp = f.body.iter().position(|s| matches!(s, Stmt::ForRange { .. }));
    let unlock = f.body.iter().position(|s| matches!(s, Stmt::MapUnlock(..)));
    assert!(lock < lp && lp < unlock, "{:#?}", f.body);
    let ret = all.iter().position(|s| matches!(s, Stmt::Return(Some(_)))).unwrap();
    assert!(matches!(all[ret - 1], Stmt::MapUnlock(..)), "{:#?}", f.body);
}

#[test]
fn copying_a_map_dups_it_and_copies_heap_values() {
    let m = lower_src("main() {\n var a: map[int, [int]] = {}\n let b = a\n}\n");
    let copies: Vec<&Func> = m.funcs.iter().filter(|f| f.name.starts_with("__kestrel_copy_")).collect();
    let map_copy = copies.iter().find(|f| matches!(f.body.first(), Some(Stmt::Dup(_))) && f.body.len() == 2);
    assert!(map_copy.is_some(), "{copies:#?}");
}

#[test]
fn printing_a_map_gets_a_printer() {
    let m = lower_src("main() {\n let a = {1: \"x\"}\n print(a)\n}\n");
    assert!(m.print_fns.keys().any(|k| matches!(k, KTy::Map(..))));
}

#[test]
fn a_loop_over_a_map_that_cant_change_doesnt_lock_it() {
    // Many `par for` iterations may read one map at once; nothing to lock.
    let m = lower_src("f(m: map[int, int]) -> int {\n var t = 0\n for k, v in m {\n t += v\n }\n t\n}\nmain() {}\n");
    let f = func(&m, "f");
    assert!(!map_ops(&f.body).contains(&MapOp::Lock), "{:#?}", f.body);
    assert!(!all_stmts(&f.body).iter().any(|s| matches!(s, Stmt::MapUnlock(..))));
}

#[test]
fn only_string_keys_that_may_be_place_reads_are_flagged_for_copying() {
    let m = lower_src(
        "main() {\n var m = map[string, int]()\n var im = map[int, int]()\n let k = \"x\"\n let c = true\n m[k] = 1\n m[\"lit\"] = 2\n m[\"a{c}\"] = 3\n m[if c { \"p\" } else { \"q\" }] = 4\n m[if c { k } else { \"q\" }] = 5\n let lit = {\"a\": 1}\n let by_var = {k: 1}\n im[7] = 1\n}\n",
    );
    let main = func(&m, "main");
    let flags: Vec<bool> = all_stmts(&main.body).into_iter().filter_map(|s| if let Stmt::MapSet(.., copy) = s { Some(*copy) } else { None }).collect();
    // A variable or an `if` with one in a branch is copied when it adds an entry; literals, interpolations, `if`s of literals and integer keys are not.
    assert_eq!(flags, [true, false, false, false, true, false, true, false], "{:#?}", main.body);
}
