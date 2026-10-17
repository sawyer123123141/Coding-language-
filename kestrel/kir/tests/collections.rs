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

fn helpers<'m>(m: &'m Module, prefix: &str) -> Vec<&'m Func> {
    m.funcs.iter().filter(|f| f.name.starts_with(prefix)).collect()
}

#[test]
fn copying_a_place_calls_the_copy_helper_but_fresh_values_dont() {
    let m = lower_src("main() {\n let a = [1, 2]\n let b = a\n let c = array[int](3)\n}\n");
    let copies = helpers(&m, "__kestrel_copy_");
    assert_eq!(copies.len(), 1);
    // copy([int]) duplicates the buffer; ints need nothing more.
    assert!(matches!(&copies[0].body[..], [Stmt::Dup(_)]));
    let main = func(&m, "main");
    let [Stmt::Let(_, a), Stmt::Let(_, b), Stmt::Let(_, c)] = &main.body[..] else { panic!("{:#?}", main.body) };
    assert!(matches!(a.kind, ExprKind::ArrayLit(_)));
    assert!(matches!(&b.kind, ExprKind::Seq(stmts, _) if matches!(stmts[..], [Stmt::Let(..), Stmt::Call(..)])));
    assert!(matches!(c.kind, ExprKind::ArrayNew(_)));
    assert_eq!(m.size_align(a.ty), (16, 8));
}

#[test]
fn nested_collections_copy_each_element() {
    let m = lower_src("struct Chunk {\n heights: [int]\n}\nmain() {\n var w = list[Chunk]()\n let v = w\n}\n");
    let copies = helpers(&m, "__kestrel_copy_");
    // list[Chunk] loops over its elements, copying each `heights`.
    assert_eq!(copies.len(), 2);
    let outer = copies.iter().find(|f| matches!(f.locals[0].ty, KTy::List(_))).unwrap();
    assert!(matches!(&outer.body[..], [Stmt::Dup(_), Stmt::ForRange { .. }]));
}

#[test]
fn recursive_types_get_one_helper_each() {
    let src = "struct Node {\n kids: [Node]\n v: int\n}\nmain() {\n let n = Node([], 1)\n let m = n\n print(m)\n}\n";
    let m = lower_src(src);
    assert_eq!(helpers(&m, "__kestrel_copy_").len(), 1);
    assert_eq!(helpers(&m, "__kestrel_print_").len(), 1);
    assert_eq!(m.print_fns.len(), 1);
}

#[test]
fn returning_a_local_moves_but_returning_a_parameter_copies() {
    let m = lower_src("mk() -> [int] {\n let a = [1]\n a\n}\nsame(xs: [int]) -> [int] {\n xs\n}\nmain() {}\n");
    let [.., Stmt::Return(Some(v))] = &func(&m, "mk").body[..] else { panic!() };
    assert!(matches!(v.kind, ExprKind::Load(_)));
    let [Stmt::Return(Some(v))] = &func(&m, "same").body[..] else { panic!() };
    assert!(matches!(v.kind, ExprKind::Seq(..)));
}

#[test]
fn for_in_is_a_counted_loop_over_an_element_place() {
    let m = lower_src("main() {\n var xs = [1, 2]\n for i, v in xs {\n  v += i\n }\n}\n");
    let main = func(&m, "main");
    let Stmt::ForRange { end, body, .. } = &main.body[1] else { panic!("{:#?}", main.body) };
    assert!(matches!(&end.kind, ExprKind::Cast(len) if matches!(len.kind, ExprKind::Len(_))));
    // `v` is a pointer to `xs[i]`, found once per iteration.
    let [Stmt::Bind(ptr, elem), Stmt::Assign(place, _)] = &body[..] else { panic!("{body:#?}") };
    assert!(matches!(elem.path[..], [Proj::Index { checked: false, .. }]));
    assert_eq!((place.root, place.path.len()), (*ptr, 0));
}

#[test]
fn for_in_reindexes_when_the_body_may_move_the_elements() {
    let src = "struct S {
 xs: list[int]
}
main() {
 var s = S([1])
 for x in s.xs {
  s.xs.add(x)
 }
 for x in s.xs {
  s = S([])
 }
}
";
    let m = lower_src(src);
    let main = func(&m, "main");
    for s in &main.body[1..] {
        let Stmt::ForRange { body, .. } = s else { panic!("{s:#?}") };
        assert!(!matches!(body[0], Stmt::Bind(..)), "{body:#?}");
    }
}

#[test]
fn list_add_and_len() {
    let m = lower_src("main() {\n var l = list[int]()\n l.add(4)\n print(l.len)\n}\n");
    let main = func(&m, "main");
    assert!(matches!(main.body[1], Stmt::ListAdd(..)));
    let Stmt::Print(args) = &main.body[2] else { panic!() };
    assert!(matches!(&args[0], PrintArg::Value(Expr { kind: ExprKind::Cast(_), ty: KTy::I32, .. })));
}

const POOLS: &str = "struct Pool {
 mem: Arena = Arena()
 n: int
}
";

/// The statements of `main` (nested ones included), outermost first.
fn all_stmts(f: &Func) -> Vec<&Stmt> {
    let mut out = Vec::new();
    kestrel_kir::visit::walk(&f.body, &mut |n| {
        if let kestrel_kir::visit::Node::Stmt(s) = n {
            out.push(s);
        }
    });
    out
}

#[test]
fn remove_at_drops_an_owning_item_once_before_the_runtime_removes_it() {
    let m = lower_src(&format!("{POOLS}main() {{\n var ps: list[Pool] = []\n ps.add(Pool(n: 1))\n ps.remove_at(0)\n}}\n"));
    let main = func(&m, "main");
    let at = main.body.iter().position(|s| matches!(s, Stmt::List(ListOp::RemoveAt, ..))).expect("a remove");
    // The item's arena is freed (once) just before, through a checked index into the list.
    let Stmt::FreeArena(arena) = &main.body[at - 1] else { panic!("{:#?}", main.body) };
    assert!(matches!(arena.path[..], [Proj::Index { checked: true, .. }, Proj::Field(0)]), "{arena:?}");
    let frees = all_stmts(main).into_iter().filter(|s| matches!(s, Stmt::FreeArena(_))).count();
    // One for the removed item; the list's own drop at the end of `main` is a call to the drop helper.
    assert_eq!(frees, 1, "{:#?}", main.body);
}

#[test]
fn remove_at_of_a_plain_item_just_removes() {
    let m = lower_src("main() {\n var xs: list[int] = [1, 2]\n xs.remove_at(0)\n}\n");
    let main = func(&m, "main");
    assert!(matches!(main.body[1], Stmt::List(ListOp::RemoveAt, ..)), "{:#?}", main.body);
    assert_eq!(main.body.len(), 2);
}

#[test]
fn pop_moves_the_item_out_and_drops_nothing_inside_the_list_op() {
    let m = lower_src(&format!("{POOLS}main() {{\n var ps: list[Pool] = []\n ps.add(Pool(n: 1))\n let p = ps.pop()\n}}\n"));
    let main = func(&m, "main");
    let stmts = all_stmts(main);
    let moves: Vec<_> = stmts
        .iter()
        .filter_map(|s| match s {
            Stmt::Assign(_, Expr { kind: ExprKind::Some(inner), .. }) => Some(inner),
            _ => None,
        })
        .collect();
    assert_eq!(moves.len(), 1, "{:#?}", main.body);
    assert!(matches!(&moves[0].kind, ExprKind::Move(p) if matches!(p.path[..], [Proj::Index { checked: true, .. }])), "{moves:?}");
    // The move zeroes the slot it vacates (`Move` does, for every item that holds an arena), then
    // the length goes down. The move is all that happens to the item: nothing is freed or removed
    // by the runtime there (the `let` drops what it holds at the end of `main`).
    let Some(Stmt::If { then_body, .. }) = stmts.iter().find(|s| matches!(s, Stmt::If { then_body, .. } if then_body.iter().any(|s| matches!(s, Stmt::SetLen(..))))) else {
        panic!("{:#?}", main.body)
    };
    assert!(matches!(&then_body[..], [Stmt::Assign(..), Stmt::SetLen(..)]), "{then_body:#?}");
    assert!(!stmts.iter().any(|s| matches!(s, Stmt::List(..))), "{:#?}", main.body);
}

#[test]
fn clear_drops_the_items_then_empties_the_list() {
    let m = lower_src(&format!("{POOLS}main() {{\n var ps: list[Pool] = []\n ps.add(Pool(n: 1))\n ps.clear()\n}}\n"));
    let main = func(&m, "main");
    // The runtime's clear zeroes every item and sets the length, so no freed arena stays reachable.
    let at = main.body.iter().position(|s| matches!(s, Stmt::List(ListOp::Clear, _, args) if args.is_empty())).expect("a clear");
    assert!(matches!(&main.body[at - 1], Stmt::Call(f, _) if m.funcs[f.0 as usize].name.starts_with("__kestrel_drop_")), "{:#?}", main.body);
}

#[test]
fn insert_evaluates_its_value_before_the_items_move() {
    let m = lower_src("main() {\n var xs: list[int] = [1, 2, 3]\n xs.insert(0, xs[2])\n}\n");
    let main = func(&m, "main");
    let at = main.body.iter().position(|s| matches!(s, Stmt::List(ListOp::Insert, ..))).expect("an insert");
    let Stmt::List(_, _, args) = &main.body[at] else { unreachable!() };
    // Both arguments are loads of temporaries set before the insert.
    assert!(args.iter().all(|a| matches!(&a.kind, ExprKind::Load(p) if p.path.is_empty())), "{args:?}");
    assert!(matches!(&main.body[at - 1], Stmt::Let(_, Expr { kind: ExprKind::Load(p), .. }) if !p.path.is_empty()), "{:#?}", main.body);
}

#[test]
fn has_and_find_call_one_search_per_list_type() {
    let m = lower_src("main() {\n let a = [1, 2]\n let b = [3]\n print(a.has(1), a.find(2), b.find(3))\n}\n");
    assert_eq!(helpers(&m, "__kestrel_find_").len(), 1);
    let search = helpers(&m, "__kestrel_find_")[0];
    assert_eq!(search.params.len(), 2);
    assert!(matches!(search.ret, Some(KTy::Optional(_))));
}

#[test]
fn every_list_change_is_reported_as_a_write_of_the_list() {
    // `for ... in` finds out from these reports that the list may move its items.
    for change in ["xs.remove_at(0)", "xs.insert(0, 1)", "xs.clear()", "xs.pop()", "xs.add(1)"] {
        let m = lower_src(&format!("main() {{\n var xs: list[int] = [1, 2, 3]\n {change}\n}}\n"));
        let main = func(&m, "main");
        let mut writes = 0;
        kestrel_kir::visit::places(&main.body, &mut |p, written| {
            writes += usize::from(written && p.path.is_empty() && p.root == LocalIdx(0));
        });
        assert!(writes > 0, "{change}: {:#?}", main.body);
    }
    // And a read-only search isn't one.
    let m = lower_src("main() {\n var xs: list[int] = [1, 2, 3]\n print(xs.has(2), xs.find(3))\n}\n");
    let mut writes = 0;
    kestrel_kir::visit::places(&func(&m, "main").body, &mut |p, written| writes += usize::from(written && p.root == LocalIdx(0)));
    assert_eq!(writes, 0);
}

#[test]
fn a_discarded_pop_of_an_owning_item_is_held_and_dropped() {
    let m = lower_src(&format!("{POOLS}main() {{\n var ps: list[Pool] = []\n ps.add(Pool(n: 1))\n ps.pop()\n}}\n"));
    let main = func(&m, "main");
    // The popped optional goes in a hidden local, and the end of `main` frees the arena in its payload.
    let frees: Vec<_> = all_stmts(main).into_iter().filter(|s| matches!(s, Stmt::FreeArena(p) if p.path.first() == Some(&Proj::Payload))).collect();
    assert_eq!(frees.len(), 1, "{:#?}", main.body);
}

/// The statements of `f`'s loops' bodies and their nested statements, with the `ForRange` that holds a `List(Compact)` after it.
fn removing_loop(f: &Func) -> (&Vec<Stmt>, usize) {
    let at = f.body.iter().position(|s| matches!(s, Stmt::List(ListOp::Compact, ..))).expect("a compaction after the loop");
    let Stmt::ForRange { body, .. } = &f.body[at - 1] else { panic!("{:#?}", f.body) };
    (body, at)
}

#[test]
fn remove_of_an_owning_item_drops_it_once() {
    let m = lower_src(&format!("{POOLS}main() {{\n var ps: list[Pool] = []\n ps.add(Pool(n: 1))\n for p in ps {{ if p.n > 0 {{ remove p }} }}\n}}\n"));
    let main = func(&m, "main");
    let frees = all_stmts(main).into_iter().filter(|s| matches!(s, Stmt::FreeArena(_))).count();
    // One for the removed item; the list's own drop at the end of `main` is a call to the drop helper.
    assert_eq!(frees, 1, "{:#?}", main.body);
    // The item is moved out of its slot (zeroing it) before it is dropped.
    let moved = all_stmts(main).into_iter().any(|s| {
        matches!(s, Stmt::Let(_, Expr { kind: ExprKind::Move(p), .. }) if matches!(p.path[..], [Proj::Index { checked: true, .. }]))
    });
    assert!(moved, "{:#?}", main.body);
}

#[test]
fn a_removing_loop_reindexes_its_item_and_compacts_after() {
    let m = lower_src("main() {\n var xs: list[int] = [1, 2, 3]\n for x in xs { if x == 2 { remove x } }\n print(xs)\n}\n");
    let main = func(&m, "main");
    let (body, at) = removing_loop(main);
    assert!(!all_stmts(main).iter().any(|s| matches!(s, Stmt::Bind(..))), "{:#?}", main.body);
    // Each pass starts with `removed = false` and ends with the keep step.
    assert!(matches!(&body[0], Stmt::Assign(_, Expr { kind: ExprKind::Bool(false), .. })), "{body:#?}");
    let Stmt::List(_, list, args) = &main.body[at] else { unreachable!() };
    assert_eq!((list.root, list.path.len(), args.len()), (LocalIdx(0), 0, 2));
}

#[test]
fn every_way_out_of_a_removing_loop_compacts_the_list() {
    let src = "f(var xs: list[int]) -> int {\n for x in xs {\n  if x == 3 { return 3 }\n  if x == 2 { remove x; break }\n  for k from 0 to 2 { if k == 1 { break } }\n }\n 0\n}\nmain() {}\n";
    let m = lower_src(src);
    let f = func(&m, "f");
    let compacts = all_stmts(f).into_iter().filter(|s| matches!(s, Stmt::List(ListOp::Compact, ..))).count();
    // After the loop (its end and `break`), and before the `return`.
    assert_eq!(compacts, 2, "{:#?}", f.body);
}

#[test]
fn promote_leaves_a_removing_loops_item_in_memory() {
    let src = "struct P {\n x: int\n y: int\n}\nmain() {\n var ps: list[P] = [P(x: 1, y: 2), P(x: 3, y: 4)]\n for p in ps {\n  p.x += 1\n  if p.y > 2 { remove p }\n }\n print(ps)\n}\n";
    let (p, d) = kestrel_syntax::parse(src);
    assert!(d.is_empty());
    let (t, d) = kestrel_check::check(&p);
    assert!(d.is_empty(), "{:?}", d.iter().map(|d| &d.message).collect::<Vec<_>>());
    let mut m = lower(&p, &t.unwrap(), &LineIndex::new(src));
    optimize(&mut m);
    let main = func(&m, "main");
    let (body, _) = removing_loop(main);
    assert!(!all_stmts(main).iter().any(|s| matches!(s, Stmt::Bind(..))), "{:#?}", main.body);
    // `p.x += 1` writes the list's memory directly: nothing caches the item in a local.
    let mut writes = 0;
    kestrel_kir::visit::places(body, &mut |p, written| {
        writes += usize::from(written && matches!(p.path[..], [Proj::Index { .. }, Proj::Field(0)]));
    });
    assert!(writes >= 1, "{body:#?}");
}
