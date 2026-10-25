//! Generics in KIR: copies lowered with their own types, generic originals as trapping stubs, and
//! placeholder layouts for the types that hold a placeholder.

use kestrel_check::Tables;
use kestrel_kir::*;
use kestrel_syntax::{parse, LineIndex};

fn checked(src: &str) -> (kestrel_syntax::ast::Program, Tables) {
    let (p, d) = parse(src);
    assert!(d.is_empty(), "parse: {:?}", d.iter().map(|d| &d.message).collect::<Vec<_>>());
    let (t, d) = kestrel_check::check(&p);
    assert!(d.is_empty(), "check: {:?}", d.iter().map(|d| &d.message).collect::<Vec<_>>());
    (p, t.expect("tables of a clean check"))
}

fn lowered(src: &str) -> Module {
    let (p, t) = checked(src);
    lower(&p, &t, &LineIndex::new(src))
}

fn funcs<'m>(m: &'m Module, name: &str) -> Vec<&'m Func> {
    m.funcs.iter().filter(|f| f.name == name).collect()
}

const FIRST: &str = "pure first[T](xs: list[T]) -> T { xs[0] }
main() {
    let a: list[int] = [1]
    let b: list[vec2] = [vec2(1, 2)]
    print(first(a), first(b))
}
";

#[test]
fn a_copy_is_lowered_with_its_types() {
    let m = lowered(FIRST);
    let ints = funcs(&m, "first[int]");
    assert_eq!(ints.len(), 1, "one `first[int]`");
    assert_eq!(ints[0].ret, Some(KTy::I32));
    assert_eq!(ints[0].params.len(), 1);
    assert!(!ints[0].exported);
    let vecs = funcs(&m, "first[vec2]");
    assert_eq!(vecs.len(), 1, "one `first[vec2]`");
    assert_eq!(vecs[0].ret, Some(KTy::Struct(StructIdx(0))));
    assert_eq!(m.layout(StructIdx(0)).name, "vec2");
    // The copy's body reads its element as the copy's type.
    let mut read = None;
    visit::walk(&ints[0].body, &mut |n| {
        if let visit::Node::Stmt(Stmt::Return(Some(v))) = n {
            read = Some(v.ty);
        }
    });
    assert_eq!(read, Some(KTy::I32));
}

#[test]
fn the_original_is_a_stub() {
    let m = lowered(FIRST);
    let stubs = funcs(&m, "first");
    assert_eq!(stubs.len(), 1);
    let f = stubs[0];
    assert!(f.params.is_empty());
    assert!(f.locals.is_empty());
    assert_eq!(f.ret, None);
    assert_eq!(f.body, vec![Stmt::Trap]);
    assert!(!f.exported);
    assert_eq!(f.lines, (0, 0));
}

#[test]
fn placeholder_layouts_line_up() {
    let src = "struct Stack[T] {
    items: list[T] = []
}
main() {
    let s: Stack[int] = Stack(items: [4])
    print(s.items.len)
}
";
    let (p, t) = checked(src);
    let m = lower(&p, &t, &LineIndex::new(src));
    assert_eq!(m.structs.len(), t.structs.len());
    assert_eq!(m.enums.len(), t.enums.len());
    assert_eq!(m.elems.len(), t.elems.len());
    let by_name = |name: &str| t.structs.iter().position(|s| s.name == name).unwrap_or_else(|| panic!("no struct {name}"));
    let inst = m.layout(StructIdx(by_name("Stack[int]") as u32));
    assert_eq!(inst.fields.len(), 1);
    assert_eq!(inst.fields[0].name, "items");
    let decl = m.layout(StructIdx(by_name("Stack") as u32));
    assert!(decl.fields.is_empty(), "the declaration's layout is the empty placeholder");
    assert_eq!((decl.size, decl.align), (0, 1));
}

#[test]
fn a_method_copy_lowers_self_as_the_instance() {
    let src = "struct Stack[T] {
    items: list[T] = []
    pure size(self) -> int { self.items.len }
}
main() {
    let s: Stack[int] = Stack(items: [4])
    print(s.size())
}
";
    let (p, t) = checked(src);
    let m = lower(&p, &t, &LineIndex::new(src));
    let inst = t.structs.iter().position(|s| s.name == "Stack[int]").expect("the instance");
    let size = funcs(&m, "Stack[int].size");
    assert_eq!(size.len(), 1);
    assert_eq!(size[0].locals[0].ty, KTy::Struct(StructIdx(inst as u32)));
    let stub = funcs(&m, "Stack.size");
    assert_eq!(stub.len(), 1);
    assert_eq!(stub[0].body, vec![Stmt::Trap]);
}
