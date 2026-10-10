use kestrel_check::{check, Ty};
use kestrel_syntax::parse;

#[test]
fn string_values_check() {
    let src = "let GREETING = \"hello\"\nstruct Named {\n name: string\n age: int\n}\npure pick(a: string, b: string, first: bool) -> string {\n if first { a } else { b }\n}\nmain() {\n let who = \"Kestrel\"\n let n = Named(pick(GREETING, who, true), 3)\n let hp = 7\n print(\"{n.name} has {hp} hp\", who)\n}\n";
    let (p, d) = parse(src);
    assert!(d.is_empty(), "{:?}", d.iter().map(|d| &d.message).collect::<Vec<_>>());
    let (t, d) = check(&p);
    assert!(d.is_empty(), "{:?}", d.iter().map(|d| &d.message).collect::<Vec<_>>());
    let t = t.unwrap();
    assert_eq!(t.locals.iter().find(|l| l.name == "who").unwrap().ty, Ty::Str);
    assert_eq!(t.consts[0].ty, Ty::Str);
}

#[test]
fn interpolation_is_a_string_anywhere() {
    let src = "main() {\n let hp = 3\n let s = \"hp {hp}/10\"\n var names = list[string]()\n names.add(\"a{hp}\")\n let same = \"{s}!\" == s\n}\n";
    let (p, d) = parse(src);
    assert!(d.is_empty(), "{:?}", d.iter().map(|d| &d.message).collect::<Vec<_>>());
    let (t, d) = check(&p);
    assert!(d.is_empty(), "{:?}", d.iter().map(|d| &d.message).collect::<Vec<_>>());
    let t = t.unwrap();
    assert_eq!(t.locals.iter().find(|l| l.name == "s").unwrap().ty, Ty::Str);
    assert!(t.has_heap(Ty::Str), "strings may live in an arena");
}
