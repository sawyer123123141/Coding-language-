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

fn idx(m: &Module, name: &str) -> FuncIdx {
    FuncIdx(m.funcs.iter().position(|f| f.name == name).unwrap_or_else(|| panic!("no function {name}")) as u32)
}

/// The functions `stmts` call directly, and whether they make a virtual call.
fn calls(stmts: &[Stmt]) -> (Vec<FuncIdx>, bool) {
    let (mut direct, mut virtual_call) = (Vec::new(), false);
    visit::walk(stmts, &mut |n| match n {
        visit::Node::Expr(Expr { kind: ExprKind::Call(f, _), .. }) | visit::Node::Stmt(Stmt::Call(f, _, _)) => direct.push(*f),
        visit::Node::Expr(Expr { kind: ExprKind::CallVirtual { .. }, .. }) | visit::Node::Stmt(Stmt::CallVirtual { .. }) => virtual_call = true,
        _ => {}
    });
    (direct, virtual_call)
}

/// Whether `stmts` make an interface value.
fn makes_iface(stmts: &[Stmt]) -> bool {
    let mut found = false;
    visit::walk(stmts, &mut |n| found |= matches!(n, visit::Node::Expr(Expr { kind: ExprKind::MakeIface { .. }, .. })));
    found
}

const RULES: &str = include_str!("../../tests/programs/generic_rules.kes");
const BEST: &str = include_str!("../../tests/programs/generic_best.kes");
const IFACE_IMPL: &str = include_str!("../../tests/programs/generic_iface_impl.kes");
const TYPE_CALLS: &str = include_str!("../../tests/programs/generic_type_calls.kes");

#[test]
fn calls_on_a_placeholder_are_direct() {
    let m = lowered(RULES);
    let (direct, virtual_call) = calls(&funcs(&m, "total_hp[Goblin]")[0].body);
    assert!(direct.contains(&idx(&m, "Goblin.hp")), "`total_hp[Goblin]` calls `Goblin.hp`");
    assert!(!virtual_call, "`total_hp[Goblin]` makes no virtual call");
    let (_, virtual_call) = calls(&funcs(&m, "total_hp[Damageable]")[0].body);
    assert!(virtual_call, "`total_hp[Damageable]` calls through the interface");
    // `var self` through a rule: `hurt[Troll]` calls `Troll.hit` directly.
    let (direct, _) = calls(&funcs(&m, "hurt[Troll]")[0].body);
    assert!(direct.contains(&idx(&m, "Troll.hit")));
}

#[test]
fn calls_reach_copies() {
    let m = lowered(BEST);
    let (direct, _) = calls(&funcs(&m, "main")[0].body);
    assert!(direct.contains(&idx(&m, "best[Enemy]")), "`main` calls `best[Enemy]`");
    assert!(direct.contains(&idx(&m, "best[int]")), "`main` calls `best[int]`");
    let stub = idx(&m, "best");
    for f in &m.funcs {
        assert!(!calls(&f.body).0.contains(&stub), "`{}` calls the stub `best`", f.name);
    }
}

#[test]
fn one_table_per_instance() {
    let m = lowered(IFACE_IMPL);
    let mut names: Vec<&str> = m.tables.iter().flat_map(|t| t.methods.iter().map(|f| m.funcs[f.0 as usize].name.as_str())).collect();
    names.sort_unstable();
    assert_eq!(names, ["Stack[float].name", "Stack[int].name", "Stack[string].name"]);
}

#[test]
fn an_annotated_generic_judges_its_copies() {
    let m = lowered("@prove(no_alloc)\npure id[T](x: T) -> T { x }\nmain() { print(id(1)) }\n");
    assert_eq!(m.contracts.len(), 1, "one site");
    assert_eq!(m.contracts[0].target, ProveTarget::Func(idx(&m, "id[int]")));
}

#[test]
fn a_conversion_to_its_own_interface_is_dropped() {
    // G14: `pass_on[Goblin]` makes `x` a `Damageable`; in `pass_on[Damageable]` it already is one.
    let src = "interface Damageable {
    pure hp(self) -> int
}
struct Goblin: Damageable {
    health: int
    pure hp(self) -> int { self.health }
}
show(d: Damageable) { print(d.hp()) }
pass_on[T: Damageable](x: T) { show(x) }
main() {
    pass_on(Goblin(health: 1))
    let d: Damageable = Goblin(health: 2)
    pass_on(d)
}
";
    let m = lowered(src);
    assert!(makes_iface(&funcs(&m, "pass_on[Goblin]")[0].body));
    let body = &funcs(&m, "pass_on[Damageable]")[0].body;
    assert!(!makes_iface(body), "the value is passed through unconverted");
    assert!(calls(body).0.contains(&idx(&m, "show")));
}

#[test]
fn written_type_calls_are_calls_and_constructions() {
    // P28: `Stack[int]()` builds a struct (not a list), and `fill[int](var s, 1, 2)` as a
    // statement calls the copy.
    let m = lowered(TYPE_CALLS);
    let main = &funcs(&m, "main")[0].body;
    let mut lists_of_structs = 0;
    visit::walk(main, &mut |n| {
        if let visit::Node::Expr(Expr { kind: ExprKind::ListNew, ty: KTy::Struct(_), .. }) = n {
            lists_of_structs += 1;
        }
    });
    assert_eq!(lists_of_structs, 0);
    let (direct, _) = calls(main);
    assert!(direct.contains(&idx(&m, "fill[int]")) && direct.contains(&idx(&m, "fill[string]")));
    assert!(direct.contains(&idx(&m, "first[int]")));
}

/// The functions named `__kestrel_closure_*`.
fn closure_codes(m: &Module) -> Vec<&Func> {
    m.funcs.iter().filter(|f| f.name.starts_with("__kestrel_closure_")).collect()
}

#[test]
fn one_closure_code_per_copy() {
    let src = "pure tag_all[T](xs: list[T], t: T) -> list[T] { xs.map((x) { t }) }
main() {
    let a: list[int] = [1]
    let b: list[string] = [\"a\"]
    print(tag_all(a, 2), tag_all(b, \"z\"))
}
";
    let m = lowered(src);
    let mut rets: Vec<Option<KTy>> = closure_codes(&m).iter().map(|f| f.ret).collect();
    rets.sort_by_key(|r| format!("{r:?}"));
    assert_eq!(rets, [Some(KTy::I32), Some(KTy::Str)], "one closure code per copy, each in its copy's types");
}

const CLOSURES: &str = include_str!("../../tests/programs/generic_closures.kes");

#[test]
fn a_default_closure_per_instance() {
    let src = CLOSURES.replace(
        "main() {",
        "main() {\n    let first = Box[int]()\n    let second = Box[string]()\n    print(first.rate(1), second.rate(\"s\"))",
    );
    let m = lowered(&src);
    // The `rate` defaults are the closures whose body is the constant 1.
    let defaults: Vec<&Func> = closure_codes(&m)
        .into_iter()
        .filter(|f| f.params.len() == 2 && matches!(f.body.as_slice(), [Stmt::Return(Some(Expr { kind: ExprKind::Int(1), .. }))]))
        .collect();
    let mut items: Vec<KTy> = defaults.iter().map(|f| f.locals[1].ty).collect();
    items.sort_by_key(|t| format!("{t:?}"));
    assert_eq!(items, [KTy::I32, KTy::Str], "one `rate` default for `Box[int]`, one for `Box[string]`");
}

#[test]
fn a_plain_default_closure_is_shared_by_every_copy() {
    // `Cfg` isn't generic, so its default means the same thing in every copy: one code.
    let src = "struct Cfg {
    rate: pure (int) -> int = (x) { x + 1 }
}
pure make[T](v: T) -> int {
    let c = Cfg()
    c.rate(1)
}
main() {
    let c = Cfg()
    print(make(1), make(\"a\"), c.rate(2))
}
";
    let m = lowered(src);
    assert_eq!(closure_codes(&m).len(), 1, "the plain default is lowered once");
}

#[test]
fn a_loop_site_per_copy() {
    let src = "pure count[T](xs: list[T]) -> int {
    var n = 0
    @prove(no_alloc)
    for i from 0 to xs.len {
        n += 1
    }
    n
}
main() {
    let a: list[int] = [1, 2]
    let b: list[string] = [\"a\"]
    print(count(a), count(b))
}
";
    let m = lowered(src);
    let mut homes: Vec<FuncIdx> = m
        .contracts
        .iter()
        .map(|s| match s.target {
            ProveTarget::Loop { func, .. } => func,
            other => panic!("not a loop site: {other:?}"),
        })
        .collect();
    homes.sort_by_key(|f| f.0);
    assert_eq!(homes, [idx(&m, "count[int]"), idx(&m, "count[string]")], "each copy has a loop site of its own");
}
