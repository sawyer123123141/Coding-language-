//! The functions behind `from_bytes[T](b)` (`lower::bytes_read`): a claimed length is checked
//! against the bytes left before anything is made for it, a map fails on a key it already has, and
//! each type gets one function however often it is read.
use kestrel_kir::*;
use kestrel_syntax::{parse, LineIndex};

fn lower_src(src: &str) -> Module {
    let (p, d) = parse(src);
    assert!(d.is_empty(), "parse: {:?}", d.iter().map(|d| &d.message).collect::<Vec<_>>());
    let (t, d) = kestrel_check::check(&p);
    assert!(d.is_empty(), "check: {:?}", d.iter().map(|d| &d.message).collect::<Vec<_>>());
    lower(&p, &t.unwrap(), &LineIndex::new(src))
}

/// The read function whose value (its third parameter) has a type that `is` accepts.
fn read_fn(m: &Module, is: impl Fn(KTy) -> bool) -> &Func {
    m.funcs.iter().find(|f| f.name.starts_with("__kestrel_bytes_read_") && is(f.locals[2].ty)).expect("a read function")
}

fn has_mul_wrap(e: &Expr) -> bool {
    let mut found = false;
    visit::walk_expr(e, &mut |n| found |= matches!(n, visit::Node::Expr(Expr { kind: ExprKind::Binary(BinOp::MulWrap, ..), .. })));
    found
}

#[test]
fn a_claimed_length_is_checked_before_the_list_is_made() {
    let m = lower_src("main() {\n    let b: list[u8] = []\n    let xs = from_bytes[list[int]](b) else { return }\n    print(xs)\n}\n");
    let f = read_fn(&m, |t| matches!(t, KTy::List(_)));
    let bound = f.body.iter().position(|s| matches!(s, Stmt::If { cond, .. } if has_mul_wrap(cond))).expect("the bound check");
    let reserve = f.body.iter().position(|s| matches!(s, Stmt::List(ListOp::Reserve, ..))).expect("the list's room");
    assert!(bound < reserve, "the bound check (statement {bound}) comes after the room (statement {reserve})");
    // The list's room is its length: `Reserve`, then `SetLen` to the same count.
    assert!(matches!(f.body[reserve + 1], Stmt::SetLen(..)));
}

#[test]
fn a_map_fails_on_a_key_it_already_has() {
    let m = lower_src("main() {\n    let b: list[u8] = []\n    let m = from_bytes[map[string, int]](b) else { return }\n    print(m)\n}\n");
    let f = read_fn(&m, |t| matches!(t, KTy::Map(..)));
    let Some(Stmt::ForRange { body, .. }) = f.body.iter().find(|s| matches!(s, Stmt::ForRange { .. })) else { panic!("the loop over the entries") };
    let set = body.iter().position(|s| matches!(s, Stmt::MapSet(..))).expect("adding the key");
    let Stmt::If { cond, then_body, .. } = &body[set + 1] else { panic!("the check after adding the key") };
    assert!(matches!(cond.kind, ExprKind::Binary(BinOp::Eq, ..)));
    assert!(matches!(then_body[..], [Stmt::Return(Some(Expr { kind: ExprKind::Int(-1), .. }))]));
}

#[test]
fn each_type_gets_one_function() {
    let src = "struct P {\n    xs: list[string]\n    n: int\n}\nmain() {\n    let b: list[u8] = []\n    let p = from_bytes[P](b) else { return }\n    let q = from_bytes[P](b) else { return }\n    let xs = from_bytes[list[string]](b) else { return }\n    print(p.n, q.n, xs)\n}\n";
    let m = lower_src(src);
    let count = |prefix: &str| m.funcs.iter().filter(|f| f.name.starts_with(prefix)).count();
    // `from_bytes` of `P` and of `list[string]`; reads of `P` and of `list[string]` (shared).
    assert_eq!(count("__kestrel_from_bytes_"), 2);
    assert_eq!(count("__kestrel_bytes_read_"), 2);
}

#[test]
fn a_list_of_empty_structs_is_read_with_no_loop() {
    let src = "struct Unit {\n}\nmain() {\n    let b: list[u8] = []\n    let xs = from_bytes[list[list[Unit]]](b) else { return }\n    print(xs.len)\n}\n";
    let m = lower_src(src);
    let f = read_fn(&m, |t| matches!(t, KTy::List(e) if matches!(m.elem(e), KTy::Struct(_))));
    // Its items take no bytes: no bound against the bytes left (only the `i32::MAX` check), no loop.
    assert!(!f.body.iter().any(|s| matches!(s, Stmt::ForRange { .. })), "a loop over items of 0 bytes");
    assert!(!f.body.iter().any(|s| matches!(s, Stmt::If { cond, .. } if has_mul_wrap(cond))), "a bound for items of 0 bytes");
    assert!(f.body.iter().any(|s| matches!(s, Stmt::SetLen(..))));
}

#[test]
fn items_far_bigger_in_memory_than_in_bytes_grow_their_list() {
    let src = "main() {\n    let b: list[u8] = []\n    let xs = from_bytes[list[int?]](b) else { return }\n    let ys = from_bytes[[int?]](b) else { return }\n    print(xs.len, ys.len)\n}\n";
    let m = lower_src(src);
    let adds = |body: &[Stmt]| body.iter().any(|s| matches!(s, Stmt::ListAdd(..)));
    for f in [read_fn(&m, |t| matches!(t, KTy::List(_))), read_fn(&m, |t| matches!(t, KTy::Array(_)))] {
        // No room for the claimed length up front: the items are added as they are read.
        assert!(!f.body.iter().any(|s| matches!(s, Stmt::List(ListOp::Reserve, ..) | Stmt::SetLen(..))), "{}", f.name);
        let Some(Stmt::ForRange { body, .. }) = f.body.iter().find(|s| matches!(s, Stmt::ForRange { .. })) else { panic!("the loop over the items") };
        assert!(adds(body), "{}", f.name);
    }
    // The array is made, at its length, only once every item has been read.
    let f = read_fn(&m, |t| matches!(t, KTy::Array(_)));
    let made = f.body.iter().position(|s| matches!(s, Stmt::Assign(_, Expr { kind: ExprKind::ArrayNew(_), .. }))).expect("the array");
    let read = f.body.iter().position(|s| matches!(s, Stmt::ForRange { .. })).expect("the loop over the items");
    assert!(read < made);
    // Items close to their bytes in size still get their room up front.
    let m = lower_src("main() {\n    let b: list[u8] = []\n    let xs = from_bytes[list[int]](b) else { return }\n    print(xs.len)\n}\n");
    let f = read_fn(&m, |t| matches!(t, KTy::List(_)));
    assert!(f.body.iter().any(|s| matches!(s, Stmt::List(ListOp::Reserve, ..))));
}
