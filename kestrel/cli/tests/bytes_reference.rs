//! `to_bytes(x)` writes the bytes the byte format defines (the I/O round's spec, decision 2), as the
//! reference encoder in `kestrel_runtime::bytes` writes them, header and fingerprint included.
use std::fs;
use std::path::Path;
use std::process::Command;

use kestrel_runtime::bytes::{encode, fingerprint, RefValue as V};
use kestrel_runtime::checksum::{finish, mix, START};

/// Runs `src` as a program in a fresh folder and returns its output lines.
fn run(name: &str, src: &str) -> Vec<String> {
    run_project(name, src, &[])
}

/// Runs `src` as `main.kes` in a fresh folder for `name`, with `files` (name, text) beside it, and
/// returns its output lines.
fn run_project(name: &str, src: &str, files: &[(&str, &str)]) -> Vec<String> {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("bytes_reference_{name}"));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    for (file, text) in files {
        fs::write(dir.join(file), text).unwrap();
    }
    let file = dir.join("main.kes");
    fs::write(&file, src).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_kestrel")).arg("run").arg(&file).env("KESTREL_POISON", "1").output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).lines().map(str::to_string).collect()
}

/// A `list[u8]` as `print` prints it.
fn printed(bytes: &[u8]) -> String {
    let items: Vec<String> = bytes.iter().map(u8::to_string).collect();
    format!("[{}]", items.join(", "))
}

fn s(text: &str) -> V {
    V::Str(text.to_string())
}

#[test]
fn to_bytes_matches_the_reference_encoder() {
    let src = "enum Shape {
    Dot
    Circle { r: float }
}

struct P {
    a: int
    s: string
    xs: list[int]
}

struct Node {
    id: u8
    kids: list[Node]
}

enum Size {
    Unknown
    Known { w: u8, h: u8 }
}

main() {
    print(to_bytes(7))
    print(to_bytes(true))
    print(to_bytes(u8(200)))
    print(to_bytes(-1.5))
    print(to_bytes(f64(0.25)))
    print(to_bytes(i64(-5)))
    print(to_bytes(\"h\u{e9}\"))
    print(to_bytes(vec2(1, 2)))
    print(to_bytes(vec4(1, 2, 3, 4)))
    let xs: list[int] = [1, 2]
    print(to_bytes(xs))
    let fs: [f64] = [0.5, -2.0]
    print(to_bytes(fs))
    print(to_bytes({\"a\": 1, \"bc\": 2}))
    let n: int? = none
    print(to_bytes(n))
    print(to_bytes(Shape.Circle(r: 2.5)))
    print(to_bytes(P(a: -7, s: \"hi\", xs: [4, 5])))
    print(to_bytes(Node(id: 1, kids: [Node(id: 2, kids: [])])))
    let filled: int? = 9
    print(to_bytes(filled))
    // A map writes its live entries only, in insertion order; a list its length, not its room.
    var m = {\"a\": 1, \"b\": 2, \"c\": 3}
    m.remove(\"b\")
    print(to_bytes(m))
    var ys: list[int] = [1, 2, 3, 4]
    ys.remove_at(0)
    ys.remove_at(2)
    print(to_bytes(ys))
    print(to_bytes(ivec4(1, -2, 3, -4)))
    print(to_bytes(Size.Known(w: 7, h: 255)))
    print(to_bytes(\"\"))
    // A NaN with a payload (made by reading its bits) keeps it; so does the NaN `0.0 / 0.0` gives.
    var nb = to_bytes(1.5)
    nb[13] = u8(1)
    nb[14] = u8(0)
    nb[15] = u8(192)
    nb[16] = u8(127)
    let nan = from_bytes[float](nb) else { return }
    print(to_bytes(nan))
    let zero = 0.0
    let q = zero / zero
    let back = from_bytes[float](to_bytes(q)) else { return }
    print(checksum(back) == checksum(q), checksum(to_bytes(back)) == checksum(to_bytes(q)))
}
";
    let lines = run("encode", src);
    let float = |x: f32| V::Float(x);
    let want: Vec<(&str, V)> = vec![
        ("int", V::Int(7)),
        ("bool", V::Bool(true)),
        ("u8", V::U8(200)),
        ("float", float(-1.5)),
        ("f64", V::F64(0.25)),
        ("i64", V::I64(-5)),
        ("string", s("h\u{e9}")),
        // The built-in vectors are written by name; their lanes are in order.
        ("vec2", V::Struct(vec![float(1.0), float(2.0)])),
        ("vec4", V::Struct(vec![float(1.0), float(2.0), float(3.0), float(4.0)])),
        ("list[int]", V::List(vec![V::Int(1), V::Int(2)])),
        ("array[f64]", V::List(vec![V::F64(0.5), V::F64(-2.0)])),
        ("map[string,int]", V::Map(vec![(s("a"), V::Int(1)), (s("bc"), V::Int(2))])),
        ("int?", V::Opt(None)),
        ("enum Shape{Dot,Circle{r:float}}", V::Enum(1, vec![float(2.5)])),
        ("struct P{a:int,s:string,xs:list[int]}", V::Struct(vec![V::Int(-7), s("hi"), V::List(vec![V::Int(4), V::Int(5)])])),
        // A type met again inside itself is written by name only.
        ("struct Node{id:u8,kids:list[Node]}", V::Struct(vec![V::U8(1), V::List(vec![V::Struct(vec![V::U8(2), V::List(Vec::new())])])])),
        ("int?", V::Opt(Some(Box::new(V::Int(9))))),
        ("map[string,int]", V::Map(vec![(s("a"), V::Int(1)), (s("c"), V::Int(3))])),
        ("list[int]", V::List(vec![V::Int(2), V::Int(3)])),
        ("ivec4", V::Struct(vec![V::Int(1), V::Int(-2), V::Int(3), V::Int(-4)])),
        ("enum Size{Unknown,Known{w:u8,h:u8}}", V::Enum(1, vec![V::U8(7), V::U8(255)])),
        ("string", s("")),
        ("float", float(f32::from_bits(0x7FC0_0001))),
    ];
    let mut want: Vec<String> = want.iter().map(|(desc, v)| printed(&encode(v, fingerprint(desc)))).collect();
    want.push("true true".to_string());
    assert_eq!(lines, want);
}

/// The checksum `tests/programs/to_bytes_struct.kes` prints: `checksum` of the `list[u8]` the
/// reference writes for its `Game` (the list's length, then each byte).
#[test]
fn the_struct_programs_checksum_is_the_references() {
    let game = V::Struct(vec![V::Struct(vec![V::Float(1.5), V::Float(-2.0)]), V::Int(3), V::Struct(vec![V::I64(258)])]);
    let bytes = encode(&game, fingerprint("struct Game{ball:vec2,score:int,rng:struct Rng{seed:i64}}"));
    let h = bytes.iter().fold(mix(START, bytes.len() as u64), |h, &b| mix(h, u64::from(b)));
    let want = finish(h) as i64;
    let program = fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("../tests/programs/to_bytes_struct.kes")).unwrap();
    // Line by line: a Windows checkout may end its lines with `\r\n`.
    let line = format!("// expect: {want}");
    assert!(program.lines().any(|l| l == line), "to_bytes_struct.kes should expect the checksum {want}");
}

#[test]
fn the_fingerprint_names_types_without_their_module() {
    // The same shape, once from a module and once at the top level, writes the same bytes, so a
    // save written by one reads back in the other.
    let types = "struct Rng {
    seed: i64
}

pub struct Game {
    score: int
    rng: Rng
}
";
    let print = "    let g = Game(score: 3, rng: Rng(seed: 258))
    print(to_bytes(g))
    let back = from_bytes[Game](to_bytes(g)) else { return }
    print(back.score)
}
";
    let from_module = format!("use {{ Game, Rng from saves }}

main() {{
{print}");
    let public = types.replace("struct Rng", "pub struct Rng");
    let a = run_project("module_types", &from_module, &[("saves.kes", &public)]);
    let b = run("top_level_types", &format!("{types}
main() {{
{print}"));
    assert_eq!(a, b);
    let game = V::Struct(vec![V::Int(3), V::Struct(vec![V::I64(258)])]);
    assert_eq!(a[0], printed(&encode(&game, fingerprint("struct Game{score:int,rng:struct Rng{seed:i64}}"))));
}
