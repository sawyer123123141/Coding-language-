//! `to_bytes(x)` writes the bytes the byte format defines (the I/O round's spec, decision 2), as the
//! reference encoder in `kestrel_runtime::bytes` writes them, header and fingerprint included.
use std::fs;
use std::path::Path;
use std::process::Command;

use kestrel_runtime::bytes::{encode, fingerprint, RefValue as V};
use kestrel_runtime::checksum::{finish, mix, START};

/// Runs `src` as a program in a fresh folder and returns its output lines.
fn run(name: &str, src: &str) -> Vec<String> {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("bytes_reference_{name}"));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
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
    ];
    let want: Vec<String> = want.iter().map(|(desc, v)| printed(&encode(v, fingerprint(desc)))).collect();
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
    assert!(program.contains(&format!("// expect: {want}\n")), "to_bytes_struct.kes should expect the checksum {want}");
}
