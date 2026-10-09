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
    let [Stmt::Assign(place, _)] = &body[..] else { panic!("{body:#?}") };
    assert!(matches!(place.path[..], [Proj::Index(_)]));
}

#[test]
fn list_add_and_len() {
    let m = lower_src("main() {\n var l = list[int]()\n l.add(4)\n print(l.len)\n}\n");
    let main = func(&m, "main");
    assert!(matches!(main.body[1], Stmt::ListAdd(..)));
    let Stmt::Print(args) = &main.body[2] else { panic!() };
    assert!(matches!(&args[0], PrintArg::Value(Expr { kind: ExprKind::Cast(_), ty: KTy::I32, .. })));
}
