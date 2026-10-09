use kestrel_check::{check, Res, Tables, Ty};
use kestrel_syntax::parse;

fn run(src: &str) -> (Option<Tables>, Vec<String>) {
    let (p, d) = parse(src);
    assert!(d.is_empty(), "parse errors: {:?}", d.iter().map(|d| &d.message).collect::<Vec<_>>());
    let (t, d) = check(&p);
    (t, d.into_iter().map(|d| d.message).collect())
}

fn ok(src: &str) -> Tables {
    let (t, errs) = run(src);
    assert!(errs.is_empty(), "unexpected errors: {errs:?}");
    t.expect("tables")
}

const ENEMIES: &str = "enum Enemy {
    Zombie { hp: int, speed: float }
    Skeleton { hp: int, arrows: int }
    Ghost

    take_damage(var self, dmg: int) {
        match self {
            Zombie z { z.hp -= dmg }
            Skeleton s { s.hp -= dmg * 2 }
            Ghost { }
        }
    }

    pure alive(self) -> bool {
        match self {
            Zombie z { z.hp > 0 }
            Skeleton s { s.hp > 0 }
            else { true }
        }
    }
}

main() {
    var e = Enemy.Zombie(hp: 10, speed: 1.5)
    e.take_damage(3)
    let g = Enemy.Ghost
    let score = match e {
        Zombie z { z.hp }
        else { 0 }
    }
    print(e, g, score, e.alive())
}
";

#[test]
fn enums_variants_and_matches_check() {
    let t = ok(ENEMIES);
    let e = t.locals.iter().find(|l| l.name == "e").unwrap();
    assert_eq!(t.ty_name(e.ty), "Enemy");
    assert!(t.res.values().any(|r| matches!(r, Res::Variant(_, 2))), "Enemy.Ghost resolves to variant 2");
    let z = t.locals.iter().find(|l| l.name == "z" && l.mutable).expect("binding in take_damage is changeable");
    assert!(matches!(z.ty, Ty::Variant(_, 0)));
    assert!(z.alias.is_some());
    assert!(t.match_arms.values().any(|arms| arms == &vec![Some(0), Some(1), Some(2)]));
    assert!(t.match_arms.values().any(|arms| arms == &vec![Some(0), None]));
}

#[test]
fn enum_errors() {
    let en = "enum E {\n A\n B { n: int }\n C\n}\n";
    let cases: Vec<(String, &str)> = vec![
        (format!("{en}main() {{\n let e = E.A\n let n = match e {{\n A {{ 1 }}\n B {{ 2 }}\n }}\n}}\n"), "`match` on `E` doesn't handle `C`"),
        (format!("{en}main() {{\n let e = E.A\n match e {{\n A {{ }}\n A {{ }}\n else {{ }}\n }}\n}}\n"), "`A` is matched twice"),
        (format!("{en}main() {{\n let e = E.A\n match e {{\n else {{ }}\n A {{ }}\n }}\n}}\n"), "this arm can never run: the `else` above matches everything"),
        (format!("{en}main() {{\n let e = E.A\n match e {{\n D {{ }}\n else {{ }}\n }}\n}}\n"), "`E` has no variant `D`"),
        (format!("{en}main() {{\n let e = E.D\n}}\n"), "`E` has no variant `D`"),
        (format!("{en}main() {{\n let e = E.B\n}}\n"), "`B` has fields; build it with `E.B(...)`"),
        (format!("{en}main() {{\n let e = E.A(1)\n}}\n"), "`A` has no fields; write `E.A`"),
        (format!("{en}main() {{\n let e = E.B()\n}}\n"), "missing field `n` for `B`"),
        (format!("{en}main() {{\n let e = E\n}}\n"), "`E` is an enum; use one of its variants, like `E.A`"),
        ("main() {\n let n = 1\n match n {\n else { }\n }\n}\n".into(), "`match` needs an enum value, found int"),
        (format!("{en}main() {{\n let e = E.B(1)\n match e {{\n B b {{ b.n = 2 }}\n else {{ }}\n }}\n}}\n"), "can't change `e`: it was declared with `let`"),
        (format!("{en}main() {{\n let e = E.B(1)\n match e {{\n B b {{ let x = b }}\n else {{ }}\n }}\n}}\n"), "`b` stands for the fields of `E.B`; use them as `b.field`"),
        (format!("{en}main() {{\n let e = E.A\n match e {{\n A a {{ }}\n else {{ }}\n }}\n}}\n"), "`A` has no fields to bind"),
        (format!("{en}main() {{\n let e = E.A\n let n = match e {{\n A {{ 1 }}\n else {{ }}\n }}\n}}\n"), "this branch has no value"),
        ("enum E {\n A { e: E }\n}\nmain() {}\n".into(), "enum `E` contains itself"),
        ("enum E {\n A\n A\n}\nmain() {}\n".into(), "`E` has two variants named `A`"),
        ("enum E {\n A\n A(self) {}\n}\nmain() {}\n".into(), "`E` already has a variant named `A`"),
    ];
    for (src, expected) in cases {
        let (_, errs) = run(&src);
        assert_eq!(errs, vec![expected.to_string()], "source:\n{src}");
    }
}
