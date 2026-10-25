//! Temporary (Task 2a deletes it): a program with a generic declaration is refused by the checker.

const SRC: &str = "struct Stack[T] {\n    v: T\n}\nmain() {}\n";

fn messages(d: &[kestrel_syntax::Diagnostic]) -> Vec<String> {
    d.iter().map(|d| d.message.clone()).collect()
}

#[test]
fn check_refuses_a_generic_declaration() {
    let (p, d) = kestrel_syntax::parse(SRC);
    assert!(d.is_empty());
    let (t, d) = kestrel_check::check(&p);
    assert!(t.is_none());
    assert_eq!(messages(&d), ["generics aren't supported yet"]);
}

#[test]
fn check_keeping_tables_refuses_it_too() {
    let (p, _) = kestrel_syntax::parse(SRC);
    let (_t, d) = kestrel_check::check_keeping_tables(&p, true);
    assert_eq!(messages(&d), ["generics aren't supported yet"]);
}

#[test]
fn check_refuses_a_user_type_call() {
    // `Enemy[int]()` settles to a `TypeCall` of a user name; Task 3b gives it "takes no types".
    let (p, d) = kestrel_syntax::parse("struct Enemy {\n    hp: int = 3\n}\nmain() {\n    let e = Enemy[int]()\n    print(e.hp)\n}\n");
    assert!(d.is_empty(), "{:?}", messages(&d));
    let (_t, d) = kestrel_check::check_keeping_tables(&p, true);
    assert_eq!(messages(&d), ["generics aren't supported yet"]);
}
