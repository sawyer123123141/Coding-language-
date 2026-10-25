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
