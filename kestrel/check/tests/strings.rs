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

fn run(src: &str) -> (Option<kestrel_check::Tables>, Vec<String>) {
    let (p, d) = parse(src);
    assert!(d.is_empty(), "parse errors: {:?}", d.iter().map(|d| &d.message).collect::<Vec<_>>());
    let (t, d) = check(&p);
    (t, d.into_iter().map(|d| d.message).collect())
}

fn ok(src: &str) -> kestrel_check::Tables {
    let (t, errs) = run(src);
    assert!(errs.is_empty(), "{errs:?}");
    t.unwrap()
}

#[test]
fn length_indexing_loops_and_plus_have_their_types() {
    let t = ok("main() {\n let s = \"héllo\"\n let n = s.len\n let c = s[1]\n let j = s + \"!\"\n for i, d in s { print(i, d) }\n for e in s { print(e) }\n}\n");
    let ty = |name: &str| t.locals.iter().find(|l| l.name == name).unwrap().ty;
    assert_eq!(ty("n"), Ty::Int);
    assert_eq!(ty("c"), Ty::Str);
    assert_eq!(ty("j"), Ty::Str);
    assert_eq!(ty("i"), Ty::Int);
    assert_eq!(ty("d"), Ty::Str);
    assert_eq!(ty("e"), Ty::Str);
}

#[test]
fn a_string_loop_is_not_a_list_loop() {
    // The item is a plain local: it isn't a view of `s`, and changing `s` in the body is fine.
    let t = ok("main() {\n var s = \"ab\"\n for c in s { s = \"x\"; print(c) }\n}\n");
    let c = t.locals.iter().find(|l| l.name == "c").unwrap();
    assert!(c.alias.is_none());
    assert!(!c.mutable);
}

#[test]
fn strings_are_not_places() {
    let (_, errs) = run("main() {\n var s = \"abc\"\n s[0] = \"x\"\n}\n");
    assert_eq!(errs, vec!["strings can't be changed in place; build a new one with `+` or `\"{...}\"`".to_string()]);
    let (_, errs) = run("main() {\n var s = \"abc\"\n s[0] += \"x\"\n}\n");
    assert_eq!(errs, vec!["strings can't be changed in place; build a new one with `+` or `\"{...}\"`".to_string()]);
    let (_, errs) = run("main() {\n var s = \"abc\"\n s.len = 1\n}\n");
    assert_eq!(errs, vec!["only variables, fields and elements can be assigned to".to_string()]);
}

#[test]
fn a_string_plus_needs_two_strings() {
    let (_, errs) = run("main() {\n let a = \"a\" + 1\n let b = 2 + \"b\"\n}\n");
    let msg = "`+` needs two numbers or two strings, found string and int".to_string();
    assert_eq!(errs, vec![msg, "`+` needs two numbers or two strings, found int and string".to_string()]);
    let (_, errs) = run("main() {\n var s = \"a\"\n s += 1\n s -= \"b\"\n}\n");
    assert_eq!(errs, vec!["expected string, found int".to_string(), "`-=` needs a number, found string".to_string()]);
}

#[test]
fn a_constant_cannot_join_strings() {
    let (_, errs) = run("let GREETING = \"a\" + \"b\"\nmain() {\n print(GREETING)\n}\n");
    assert_eq!(errs.len(), 1, "{errs:?}");
    assert!(errs[0].contains("can't join strings"), "{errs:?}");
}

#[test]
fn unknown_string_fields_and_methods_say_what_exists() {
    let (p, _) = parse("main() {\n let s = \"a\"\n print(s.size)\n s.reverse()\n}\n");
    let (_, d) = check(&p);
    let d: Vec<(String, Option<String>)> = d.into_iter().map(|d| (d.message, d.help)).collect();
    assert_eq!(d[0], ("string has no field `size`".to_string(), Some("its length is `.len`".to_string())));
    assert_eq!(
        d[1],
        (
            "string has no method `reverse`".to_string(),
            Some("strings have `has`, `find`, `starts_with`, `ends_with`, `slice`, `split`, `upper`, `lower`, `trim`, `to_int`, `to_float` and `.len`".to_string())
        )
    );
}

#[test]
fn string_results_are_checked_as_new_memory() {
    let (_, errs) = run("main() {\n var keep = \"\"\n arena {\n  let t = \"{1}\"\n  keep = t[0]\n }\n}\n");
    assert_eq!(errs, vec!["this memory is freed at the end of the `arena { }` block, but `keep` lives longer".to_string()]);
    let (_, errs) = run("main() {\n var keep = \"\"\n arena {\n  keep += \"x\"\n }\n}\n");
    assert_eq!(errs, vec!["this memory is freed at the end of the `arena { }` block, but `keep` lives longer".to_string()]);
    let (_, errs) = run("main() {\n var keep = \"\"\n arena {\n  keep = \"a\" + \"b\"\n }\n}\n");
    assert_eq!(errs, vec!["this memory is freed at the end of the `arena { }` block, but `keep` lives longer".to_string()]);
    // A literal belongs to no arena.
    ok("main() {\n var keep = \"\"\n arena {\n  keep = \"x\"\n }\n}\n");
}
