use kestrel_check::{check, Tables, Ty};
use kestrel_syntax::parse;

fn run(src: &str) -> (Option<Tables>, Vec<String>) {
    let (p, d) = parse(src);
    assert!(d.is_empty(), "parse errors: {:?}", d.iter().map(|d| &d.message).collect::<Vec<_>>());
    let (t, d) = check(&p);
    (t, d.into_iter().map(|d| d.message).collect())
}

fn local<'t>(t: &'t Tables, name: &str) -> &'t kestrel_check::LocalInfo {
    t.locals.iter().find(|l| l.name == name).unwrap_or_else(|| panic!("no local {name}"))
}

const PROGRAM: &str = "struct Chunk {
    x: int
    heights: [int] = []
}

pure total(xs: [int]) -> int {
    var s = 0
    for v in xs { s += v }
    s
}

main() {
    var a = [1, 2, 3]
    a[0] = 5
    a[1] += a[2]
    let e: [float] = []
    var grid = array[u8](16)
    grid[3] = 7
    var world = list[Chunk]()
    world.add(Chunk(x: 1))
    world[0].heights = array[int](4)
    world[0].heights[2] = 9
    for i, c in world {
        c.x += i
    }
    var tris: list[vec3] = []
    tris.add(vec3(1, 2, 3))
    let n = a.len + world.len + tris.len
    print(total(a), n, a, e, grid)
}
";

#[test]
fn arrays_and_lists_check() {
    let (t, errs) = run(PROGRAM);
    assert!(errs.is_empty(), "{errs:?}");
    let t = t.unwrap();
    assert_eq!(t.ty_name(local(&t, "a").ty), "[int]");
    assert_eq!(t.ty_name(local(&t, "e").ty), "[float]");
    assert_eq!(t.ty_name(local(&t, "grid").ty), "[u8]");
    assert_eq!(t.ty_name(local(&t, "world").ty), "list[Chunk]");
    assert_eq!(t.ty_name(local(&t, "tris").ty), "list[vec3]");
    assert_eq!(local(&t, "n").ty, Ty::Int);
    // The loop binding stands for `world[i]` and can change it.
    let world = t.locals.iter().position(|l| l.name == "world").unwrap();
    let c = local(&t, "c");
    assert_eq!(c.alias.map(|l| l.0 as usize), Some(world));
    assert!(c.mutable);
    assert_eq!(local(&t, "i").ty, Ty::Int);
}

#[test]
fn collection_errors() {
    let cases: &[(&str, &str)] = &[
        ("main() {\n let xs = []\n}\n", "can't tell what `[]` holds"),
        ("main() {\n let a = [1, true]\n}\n", "expected int, found bool"),
        ("main() {\n let x = 5\n let y = x[0]\n}\n", "can't index int; only arrays, lists and maps can be indexed"),
        ("main() {\n let a = [1]\n let y = a[true]\n}\n", "an index must be an integer, found bool"),
        ("main() {\n let a = [1]\n a[0] = 2\n}\n", "can't change `a`: it was declared with `let`"),
        ("main() {\n var a = [1]\n a.add(2)\n}\n", "arrays have a fixed length; use a `list[int]` to add items"),
        ("main() {\n let l = list[int]()\n l.add(1)\n}\n", "can't call `add` on `l`: it changes `l`, which was declared with `let`"),
        ("main() {\n var l = list[int]()\n l.add(true)\n}\n", "expected int, found bool"),
        ("main() {\n let a = [1]\n let n = a.size\n}\n", "`[int]` has no field `size`"),
        ("main() {\n let a = array[int]()\n}\n", "`array[T](n)` takes one length"),
        ("main() {\n let l = list[int](3)\n}\n", "`list[T]()` takes no arguments"),
        ("main() {\n let a = array[int](1.5)\n}\n", "an array length must be an integer, found float"),
        ("main() {\n for v in 5 { }\n}\n", "can't loop over int; `for ... in` needs an array, list or map"),
        ("main() {\n let a = [1]\n for v in a { v = 2 }\n}\n", "can't change `a`: it was declared with `let`"),
        ("main() {\n let list = 3\n}\n", "`list` is a built-in name; pick another"),
        ("f(xs: [int]) {\n xs[0] = 1\n}\nmain() {}\n", "can't change parameter `xs`"),
        ("main() {\n let m = map[vec3, int]()\n}\n", "map keys must be integers, bools or strings, not `vec3`"),
        ("f(x: array[int]) {}\nmain() {}\n", "write array types as `[T]`"),
        ("main() {\n var a = [1]\n a.push(2)\n}\n", "`[int]` has no method `push`"),
        ("main() {\n var l = list[int]()\n l.add()\n}\n", "`add` takes one value"),
    ];
    for (src, expected) in cases {
        let errs = run(src).1;
        assert_eq!(errs, vec![expected.to_string()], "source:\n{src}");
    }
}

#[test]
fn exclusivity_reaches_through_elements() {
    let src = "f(var x: [int], y: [[int]]) {}\nmain() {\n var a = [[1], [2]]\n f(var a[0], a)\n}\n";
    let errs = run(src).1;
    assert_eq!(errs.len(), 1, "{errs:?}");
    assert!(errs[0].contains("`a`"), "{errs:?}");
    // Reading a number out of the changed variable is fine: it's copied first.
    let ok = "g(var x: [int], n: int) {}\nmain() {\n var a = [1, 2]\n g(var a, a[0])\n}\n";
    assert_eq!(run(ok).1, Vec::<String>::new());
}
