//! Checks of generic declarations: endless copies (G10), self-containment through generics,
//! conformance for every `T`, and which types can be `T` (R3, R7).

use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use kestrel_check::{check_keeping_tables, GenericOwner, StructId, Tables, Ty};
use kestrel_syntax::parse;

/// The error messages for `src`, in order, failing (instead of hanging) when the check doesn't end.
fn errors(src: &str) -> Vec<String> {
    let owned = src.to_string();
    let (send, recv) = mpsc::channel();
    thread::spawn(move || {
        let (p, d) = parse(&owned);
        assert!(d.is_empty(), "parse: {:?}", d.iter().map(|d| &d.message).collect::<Vec<_>>());
        let (_t, d) = check_keeping_tables(&p, true);
        let _ = send.send(d.into_iter().filter(|d| d.is_error()).map(|d| d.message).collect::<Vec<_>>());
    });
    recv.recv_timeout(Duration::from_secs(30)).expect("the check finishes")
}

/// The tables and error messages for `src` (the tables come back even with errors).
fn tables_and_errors(src: &str) -> (Tables, Vec<String>) {
    let (p, d) = parse(src);
    assert!(d.is_empty(), "parse: {:?}", d.iter().map(|d| &d.message).collect::<Vec<_>>());
    let (t, d) = check_keeping_tables(&p, true);
    (t, d.into_iter().filter(|d| d.is_error()).map(|d| d.message).collect())
}

fn struct_named(t: &Tables, name: &str) -> Option<StructId> {
    t.structs.iter().position(|s| s.name == name).map(|i| StructId(i as u32))
}

const MUTUAL: &str = "struct A[T] {\n    b: list[B[T]]\n}\nstruct B[T] {\n    a: list[A[list[T]]]\n}\nmain() {}\n";

#[test]
fn a_growing_cycle_is_reported_once() {
    let errs = errors(MUTUAL);
    let endless: Vec<&String> = errs.iter().filter(|e| e.contains("would need endless copies")).collect();
    assert_eq!(endless.len(), 1, "{errs:?}");
    assert!(endless[0].starts_with("`A` would need endless copies: `A[T]` holds a `A[list[T]]`"), "{errs:?}");
    assert_eq!(errs.len(), 1, "{errs:?}");
    // Used, it still ends: the instance's fields are left as errors.
    let used = format!("{MUTUAL}f(x: A[int]) {{}}\n");
    let errs = errors(&used);
    assert_eq!(errs.len(), 1, "{errs:?}");
    assert!(errs[0].contains("would need endless copies"), "{errs:?}");
}

#[test]
fn every_declaration_of_a_growing_cycle_is_endless() {
    let (t, _) = tables_and_errors(MUTUAL);
    let a = struct_named(&t, "A").unwrap();
    let b = struct_named(&t, "B").unwrap();
    assert!(t.generic.endless.contains(&GenericOwner::Struct(a)));
    assert!(t.generic.endless.contains(&GenericOwner::Struct(b)));
}

#[test]
fn an_unused_growing_declaration_is_reported_and_ends() {
    let src = "struct Tree[T] {\n    kids: list[Tree[list[T]]]\n}\nmain() {}\n";
    let errs = errors(src);
    assert_eq!(
        errs,
        ["`Tree` would need endless copies: `Tree[T]` holds a `Tree[list[T]]`, which holds a `Tree[list[list[T]]]`, and so on"]
    );
    let (p, _) = parse(src);
    let (_, d) = check_keeping_tables(&p, true);
    assert_eq!(d[0].help.as_deref(), Some("use the same placeholders (`Tree[T]`), or types that don't grow"));
}

#[test]
fn growth_without_a_cycle_is_fine() {
    let src = "struct N[T] {\n    kids: list[N[T]]\n}\nstruct M[T] {\n    n: list[N[M[T]]]\n}\nf(x: M[int]) {}\nmain() {}\n";
    assert!(errors(src).is_empty());
}

#[test]
fn a_generic_method_must_fit_for_every_t() {
    let src = "interface Hp {\n    pure hp(self) -> int\n}\nstruct S[T]: Hp {\n    v: T\n    pure hp(self) -> T { self.v }\n}\nmain() {}\n";
    assert_eq!(errors(src), ["`S.hp` doesn't match `Hp.hp`: it should be `pure hp(self) -> int`"]);
}

#[test]
fn an_instance_never_reports_containing_itself() {
    let src = "struct Wrap[T] {\n    v: T\n}\nstruct Bad {\n    w: Wrap[Bad]\n}\nmain() {}\n";
    assert_eq!(errors(src), ["struct `Bad` contains itself"]);
    let src = "enum Maybe[T] {\n    Just { v: T }\n    Nothing\n}\nstruct Node[T] {\n    next: Maybe[Node[T]]?\n}\nmain() {}\n";
    assert_eq!(errors(src), ["struct `Node` contains itself"]);
    // A declaration holding one of its own instances in place is endless in every copy.
    let src = "struct G[T] {
    v: G[int]?
}
f(x: G[bool]) {}
main() {}
";
    assert_eq!(errors(src), ["struct `G` contains itself"]);
}

#[test]
fn purity_alone_is_no_clash() {
    let src = "interface A {\n    pure m(self) -> int\n}\ninterface B {\n    m(self) -> int\n}\npure f[T: A + B](x: T) -> int { 1 }\npure g[T: B + A](x: T) -> int { 1 }\nmain() {}\n";
    assert!(errors(src).is_empty());
    let clash = src.replace("    m(self) -> int", "    m(self) -> float");
    let errs = errors(&clash);
    assert_eq!(errs.len(), 2, "{errs:?}");
    assert!(errs[0].contains("no type can be both `A` and `B`: their `m` methods differ"), "{errs:?}");
    assert!(errs[1].contains("no type can be both `B` and `A`: their `m` methods differ"), "{errs:?}");
}

const OWNING: &str = "struct Box[T] {\n    v: T\n}\nstruct World {\n    mem: Arena\n}\nf(b: Box[World]) {}\nmain() {\n    let w: Box[Arena]? = none\n}\n";

#[test]
fn an_owning_argument_is_refused_and_makes_no_owning_instance() {
    let (t, errs) = tables_and_errors(OWNING);
    // (An annotation with an error leaves `none` untyped: today's follow-on error.)
    let owning: Vec<&String> = errs.iter().filter(|e| e.contains("owns")).collect();
    assert_eq!(owning, ["`World` owns an `Arena`, so it can't be `T` yet", "`Arena` owns an `Arena`, so it can't be `T` yet"]);
    // Named in a signature, it was interned before its arguments could be checked; it answers
    // as its declaration does, never owning.
    let in_sig = struct_named(&t, "Box[World]").expect("interned by the signature");
    assert!(!t.is_owning(Ty::Struct(in_sig)));
    assert_eq!(t.owning_structs.len(), t.structs.len());
    // Written in a body, it is refused before it is interned.
    assert!(struct_named(&t, "Box[Arena]").is_none());
}

#[test]
fn a_placeholder_meets_the_rules_it_carries() {
    let src = "interface Hp {\n    pure hp(self) -> int\n}\nstruct Eq[T: equatable] {\n    v: T\n}\nstruct Hurt[T: Hp] {\n    who: T\n}\nstruct Grid[K: key] {\n    cells: map[K, int]\n}\nf[U: ordered](x: Eq[U]) {}\ng[U: Hp + key](x: Hurt[U], y: Grid[U], z: Eq[U]) {}\nstruct Keep[V: key] {\n    g: Grid[V]\n}\nh(x: Hurt[Hp], y: Eq[vec2], z: Grid[u8]) {}\nmain() {}\n";
    assert!(errors(src).is_empty(), "{:?}", errors(src));
}

#[test]
fn a_placeholder_missing_a_rule_is_told_which_to_add() {
    let src = "interface Hp {\n    pure hp(self) -> int\n}\nstruct Hurt[T: Hp] {\n    who: T\n}\nstruct Keep[V: equatable] {\n    h: Hurt[V]\n}\nmain() {}\n";
    assert_eq!(errors(src), ["`V` isn't `Hp`, so it can't be `T` in `Hurt`; add the rule: `[V: equatable + Hp]`"]);
}

#[test]
fn types_written_in_bodies_are_checked_at_once() {
    let src = "struct Grid[K: key] {\n    cells: map[K, int]\n}\nmain() {\n    let g: list[Grid[float]] = []\n    let o: list[Grid[int?]] = []\n}\n";
    // (An annotation with an error leaves `[]` untyped: today's follow-on error.)
    let errs: Vec<String> = errors(src).into_iter().filter(|e| e.contains("can't be `K`")).collect();
    assert_eq!(
        errs,
        [
            "`float` isn't a `key` (integers, bools and strings are), so it can't be `K` in `Grid`",
            "`int?` is an optional, so it can't be `K` yet; hold it in a struct field instead",
        ]
    );
}
