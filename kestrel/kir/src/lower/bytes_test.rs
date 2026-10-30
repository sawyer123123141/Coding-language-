use kestrel_check::Tables;
use kestrel_syntax::parse;

use super::{checksum_of_text, description, fingerprint};
use crate::ir::{KTy, StructIdx};

/// The tables of a program that declares `decls`.
fn tables(decls: &str) -> Tables {
    let src = format!("{decls}\nmain() {{}}\n");
    let (p, d) = parse(&src);
    assert!(d.is_empty(), "parse errors: {:?}", d.iter().map(|d| &d.message).collect::<Vec<_>>());
    let (t, diags) = kestrel_check::check_keeping_tables(&p, true);
    assert!(diags.iter().all(|d| !d.is_error()), "errors: {:?}", diags.iter().map(|d| &d.message).collect::<Vec<_>>());
    t
}

fn strukt(t: &Tables, name: &str) -> KTy {
    KTy::Struct(StructIdx(t.structs.iter().position(|s| s.name == name).expect("the struct") as u32))
}

fn field(t: &Tables, ty: KTy, name: &str) -> KTy {
    let KTy::Struct(s) = ty else { unreachable!() };
    crate::lower::kty(t.structs[s.0 as usize].fields.iter().find(|f| f.name == name).expect("the field").ty)
}

#[test]
fn the_description_names_every_field_in_order() {
    let t = tables(
        "struct Rng {
    seed: i64
}
struct Game {
    ball: vec2
    vel: vec2
    score: int
    rng: Rng
}",
    );
    let game = strukt(&t, "Game");
    let desc = "struct Game{ball:vec2,vel:vec2,score:int,rng:struct Rng{seed:i64}}";
    assert_eq!(description(game, &t), desc);
    assert_eq!(fingerprint(game, &t), kestrel_runtime::bytes::fingerprint(desc));
}

#[test]
fn reordering_two_fields_changes_the_fingerprint() {
    let a = tables("struct P {\n    x: int\n    y: int\n}");
    let b = tables("struct P {\n    y: int\n    x: int\n}");
    let (pa, pb) = (strukt(&a, "P"), strukt(&b, "P"));
    assert_ne!(fingerprint(pa, &a), fingerprint(pb, &b));
    // Renaming one does too.
    let c = tables("struct P {\n    x: int\n    z: int\n}");
    assert_ne!(fingerprint(pa, &a), fingerprint(strukt(&c, "P"), &c));
}

#[test]
fn containers_enums_and_self_containing_types_are_described() {
    let t = tables(
        "enum State {
    Alive
    Dead { since: int }
}
struct Node {
    kids: list[Node]
}
struct All {
    s: State
    n: Node
    xs: [f64]
    m: map[string, list[u8]]
    o: bool?
    v: vec4
    w: ivec4
    q: vec3
}",
    );
    let all = strukt(&t, "All");
    let cases = [
        ("s", "enum State{Alive,Dead{since:int}}"),
        ("n", "struct Node{kids:list[Node]}"),
        ("xs", "array[f64]"),
        ("m", "map[string,list[u8]]"),
        ("o", "bool?"),
        ("v", "vec4"),
        ("w", "ivec4"),
        ("q", "vec3"),
    ];
    for (name, want) in cases {
        assert_eq!(description(field(&t, all, name), &t), want, "field `{name}`");
    }
}

#[test]
fn the_fold_equals_the_runtimes_checksum_of_a_string() {
    for text in ["", "struct Rng{seed:i64}", "a description longer than one eight-byte word"] {
        let runtime = kestrel_runtime::checksum::finish(kestrel_runtime::checksum::mix_bytes(kestrel_runtime::checksum::START, text.as_bytes()));
        assert_eq!(checksum_of_text(text.as_bytes()), runtime as i64, "{text:?}");
    }
}
