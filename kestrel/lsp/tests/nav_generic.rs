//! Hover on generic code: a call through a `T`'s rule is its interface's method.

use kestrel_driver::analyze;
use kestrel_lsp::resolve::resolve;

#[test]
fn a_call_through_a_rule_shows_the_interface_method() {
    let src = "interface Damageable {\n    pure hp(self) -> int\n}\npure total[T: Damageable](x: T) -> int {\n    x.hp()\n}\nmain() {}\n";
    let a = analyze("main.kes", src, true, &|_| None);
    assert!(a.diags.is_empty(), "{:?}", a.diags);
    let off = src.find("hp()").unwrap() as u32 + 1;
    let s = resolve(&a, off).expect("something at the call");
    assert_eq!(s.code, "pure Damageable.hp(self) -> int");
    // It leads to the method's declaration in the interface.
    assert_eq!(s.def.map(|d| d.start), Some(src.find("hp(self)").unwrap() as u32));
}

#[test]
fn a_generic_with_copies_leads_to_its_declaration() {
    let src = "pure best[T](xs: list[T]) -> T {\n    xs[0]\n}\nmain() {\n    let a: list[int] = [1]\n    let b: list[float] = [2.5]\n    print(best(a))\n    print(best(b))\n}\n";
    let a = analyze("main.kes", src, true, &|_| None);
    assert!(a.diags.is_empty(), "{:?}", a.diags);
    let t = a.tables.as_ref().expect("tables");
    let copies: Vec<&str> = t.fns.iter().filter(|f| f.copy.is_some()).map(|f| f.name.as_str()).collect();
    assert_eq!(copies, ["best[int]", "best[float]"]);
    // The copies share the generic's span; the generic comes first, so its declaration is found.
    let decl = src.find("best").unwrap() as u32;
    for off in [decl + 1, src.rfind("best(b)").unwrap() as u32 + 1] {
        let s = resolve(&a, off).expect("something at the name");
        assert_eq!(s.code, "pure best(xs: list[T]) -> T");
        assert_eq!(s.def.map(|d| d.start), Some(decl));
    }
}
