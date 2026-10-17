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

#[test]
fn string_methods_have_their_types() {
    let t = ok("main() {\n let s = \"a,b\"\n let h = s.has(\"a\")\n let st = s.starts_with(\"a\")\n let en = s.ends_with(\"b\")\n let f = s.find(\"b\")\n let sl = s.slice(0, 1)\n let sp = s.split(\",\")\n let u = s.upper()\n let l = s.lower()\n let tr = s.trim()\n let i = s.to_int()\n let x = s.to_float()\n let parts = [\"a\", \"b\"]\n let j = parts.join(\", \")\n let ls: list[string] = []\n let j2 = ls.join(\"\")\n}\n");
    let ty = |name: &str| t.locals.iter().find(|l| l.name == name).unwrap().ty;
    let opt = |ty: Ty| Ty::Optional(t.elem_ids[&ty]);
    for name in ["h", "st", "en"] {
        assert_eq!(ty(name), Ty::Bool, "{name}");
    }
    for name in ["sl", "u", "l", "tr", "j", "j2"] {
        assert_eq!(ty(name), Ty::Str, "{name}");
    }
    assert_eq!(ty("f"), opt(Ty::Int));
    assert_eq!(ty("i"), opt(Ty::Int));
    assert_eq!(ty("x"), opt(Ty::Float));
    assert_eq!(ty("sp"), Ty::List(t.elem_ids[&Ty::Str]));
}

#[test]
fn string_method_arguments_are_checked() {
    let cases = [
        ("s.slice(1)", "`slice` takes two values"),
        ("s.slice(1, 2, 3)", "`slice` takes two values"),
        ("s.find(1)", "expected string, found int"),
        ("s.split(\",\", 1)", "`split` takes one value"),
        ("s.upper(1)", "`upper` takes no arguments"),
        ("s.has()", "`has` takes one value"),
        ("s.slice(1.5, 2)", "an index must be an integer, found float"),
        ("s.trim(x: 1)", "`trim` takes no arguments"),
        ("parts.join(1)", "expected string, found int"),
        ("parts.join()", "`join` takes one value"),
    ];
    for (call, want) in cases {
        let src = format!("main() {{\n let s = \"abc\"\n let parts = [\"a\"]\n let r = {call}\n}}\n");
        let (_, errs) = run(&src);
        assert_eq!(errs, vec![want.to_string()], "{call}");
    }
}

#[test]
fn join_needs_strings() {
    let (_, errs) = run("main() {\n let a = [1, 2].join(\",\")\n let l: list[int] = []\n let b = l.join(\",\")\n}\n");
    assert_eq!(
        errs,
        vec!["`join` needs a list of strings, found `[int]`".to_string(), "`join` needs a list of strings, found `list[int]`".to_string()]
    );
}

#[test]
fn a_discarded_string_read_says_the_line_does_nothing() {
    for call in ["s.has(\"a\")", "s.find(\"a\")", "s.upper()", "s.split(\",\")", "s.to_int()", "parts.join(\",\")"] {
        let src = format!("main() {{\n let s = \"abc\"\n let parts = [\"a\"]\n {call}\n}}\n");
        let (_, errs) = run(&src);
        let want = if call.starts_with("parts") { "this only reads the list, so the line does nothing" } else { "this only reads the string, so the line does nothing" };
        assert_eq!(errs, vec![want.to_string()], "{call}");
    }
}

#[test]
fn string_method_results_are_new_memory() {
    let msg = "this memory is freed at the end of the `arena { }` block, but `keep` lives longer".to_string();
    for value in ["\"a,b\".split(\",\")[0]", "\"abc\".slice(0, 1)", "\"abc\".upper()", "\"abc\".trim()", "[\"a\", \"b\"].join(\",\")"] {
        let src = format!("main() {{\n var keep = \"\"\n arena {{\n  keep = {value}\n }}\n}}\n");
        let (_, errs) = run(&src);
        assert_eq!(errs, vec![msg.clone()], "{value}");
    }
    let (_, errs) = run("main() {\n var keep: list[string] = []\n arena {\n  keep = \"a,b\".split(\",\")\n }\n}\n");
    assert_eq!(errs, vec![msg]);
}

#[test]
fn a_constant_cannot_call_a_string_method() {
    let (_, errs) = run("let UP = \"a\".upper()\nmain() {\n print(UP)\n}\n");
    assert_eq!(errs.len(), 1, "{errs:?}");
}
