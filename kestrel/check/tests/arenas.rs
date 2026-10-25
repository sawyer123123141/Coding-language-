use kestrel_check::{check, Tables, Ty};
use kestrel_syntax::parse;

fn run(src: &str) -> (Option<Tables>, Vec<String>) {
    let (p, d) = parse(src);
    assert!(d.is_empty(), "parse errors: {:?}", d.iter().map(|d| &d.message).collect::<Vec<_>>());
    let (t, d) = check(&p);
    (t, d.into_iter().map(|d| d.message).collect())
}

const CHUNK: &str = "let SIZE = 4
struct Chunk {
    x: int
    mem: Arena = Arena()
    heights: [int] = []

    generate(var self) {
        in self.mem {
            self.heights = array[int](SIZE * SIZE)
        }
        for i from 0 to SIZE * SIZE { self.heights[i] = self.x + i }
    }
}
struct Pair {
    c: Chunk
}
pure squares(n: int) -> [int] {
    var out = array[int](n)
    for i from 0 to n { out[i] = i * i }
    out
}
";

#[test]
fn arenas_moves_and_ownership_check() {
    let src = format!(
        "{CHUNK}main() {{
    var world = list[Chunk]()
    for cx from 0 to 3 {{
        world.add(Chunk(x: cx))
    }}
    for chunk in world {{
        chunk.generate()
        in chunk.mem {{
            chunk.heights = squares(4)
        }}
    }}
    let c = Chunk(x: 9)
    world.add(c)
    var total = 0
    arena {{
        var tmp = list[int]()
        for chunk in world {{ tmp.add(chunk.heights.len) }}
        let sq = squares(3)
        var copy = sq
        for v in tmp {{ total += v }}
        total += copy.len
    }}
    print(total)
}}
"
    );
    let (t, errs) = run(&src);
    assert!(errs.is_empty(), "{errs:?}");
    let t = t.unwrap();
    let chunk = t.locals.iter().find(|l| l.name == "c").unwrap().ty;
    assert!(t.is_owning(chunk));
    assert!(t.is_owning(t.locals.iter().find(|l| l.name == "world").unwrap().ty));
    assert!(!t.is_owning(Ty::Int));
    assert_eq!(t.moves.len(), 1, "only `world.add(c)` moves");
}

#[test]
fn ownership_and_escape_errors() {
    let cases: &[(&str, &str)] = &[
        ("main() {\n let a = Chunk(x: 1)\n let b = a\n let c = a\n}\n", "`a` was moved earlier, so it can't be used here"),
        (
            "main() {\n let a = Chunk(x: 1)\n var w = list[Chunk]()\n for i from 0 to 2 { w.add(a) }\n}\n",
            "`a` is moved inside a loop, so the next time around it would already be gone",
        ),
        (
            "main() {\n let p = Pair(Chunk(x: 1))\n let c = p.c\n}\n",
            "can't move this out: it holds an `Arena`, so it can't be copied, and only whole variables move",
        ),
        (
            "f(c: Chunk) -> Chunk {\n c\n}\nmain() {}\n",
            "can't move parameter `c`: it belongs to the caller, and it holds an `Arena`, so it can't be copied",
        ),
        (
            "main() {\n var c = Chunk(x: 1)\n c.mem = Arena()\n}\n",
            "an `Arena` can't be replaced: values that point into it would be left dangling",
        ),
        (
            "main() {\n var xs = [1]\n arena {\n  xs = [2, 3]\n }\n}\n",
            "this memory is freed at the end of the `arena { }` block, but `xs` lives longer",
        ),
        (
            "main() {\n var l = list[[int]]()\n arena {\n  l.add([1])\n }\n}\n",
            "this memory is freed at the end of the `arena { }` block, but `l` lives longer",
        ),
        (
            "f() -> [int] {\n arena {\n  return [1]\n }\n [2]\n}\nmain() {}\n",
            "can't return memory from an `arena { }` block: it's freed at the block's end",
        ),
        (
            "fill(var c: Chunk) {\n c.heights = [1, 2]\n}\nmain() {\n var c = Chunk(x: 1)\n arena {\n  fill(var c)\n }\n}\n",
            "`fill` stores memory in `c` that is freed at the end of the `arena { }` block, but `c` lives longer",
        ),
        (
            "main() {\n var c = Chunk(x: 1)\n var other = [0]\n in c.mem {\n  other = [1, 2]\n }\n}\n",
            "this memory belongs to `c`, but `other` isn't part of `c`",
        ),
        ("main() {\n let x = 5\n in x { }\n}\n", "`in` needs an `Arena`, like `in self.mem { }`; found int"),
        ("main() {\n in Arena() { }\n}\n", "`in` needs an arena stored in a variable or field"),
        ("main() {\n let a = Chunk(x: 1)\n let arr = [a, a]\n}\n", "`a` was moved earlier, so it can't be used here"),
        ("main() {\n let a = Arena(1)\n}\n", "`Arena()` takes no arguments"),
        ("struct Arena {\n x: int\n}\nmain() {}\n", "`Arena` is a built-in name; pick another"),
    ];
    for (body, expected) in cases {
        let src = format!("{CHUNK}{body}");
        let errs = run(&src).1;
        assert_eq!(errs, vec![expected.to_string()], "source:\n{body}");
    }
}

#[test]
fn values_made_and_kept_inside_a_block_are_fine() {
    let ok = [
        "main() {\n arena {\n  let t = [1]\n  var u = t\n  u[0] = 2\n }\n}\n",
        "main() {\n var xs = [1]\n arena {\n  xs[0] = 5\n }\n}\n",
        "mk() -> [int] {\n let a = [1, 2]\n a\n}\nmain() {\n let b = mk()\n}\n",
        "main() {\n var c = Chunk(x: 1)\n in c.mem {\n  let local = [1, 2]\n  c.heights = local\n }\n}\n",
        // A store into a `var` parameter is checked where the function is called (P38).
        "fill(var c: Chunk) {\n c.heights = [1, 2]\n}\nmain() {\n var c = Chunk(x: 1)\n fill(var c)\n}\n",
    ];
    for body in ok {
        let errs = run(&format!("{CHUNK}{body}")).1;
        assert!(errs.is_empty(), "{errs:?}\n{body}");
    }
}

/// The help lines of the errors in `src`.
fn helps(src: &str) -> Vec<Option<String>> {
    let (p, d) = parse(src);
    assert!(d.is_empty(), "parse errors: {:?}", d.iter().map(|d| &d.message).collect::<Vec<_>>());
    check(&p).1.into_iter().filter(|d| d.is_error()).map(|d| d.help).collect()
}

#[test]
fn escape_help_names_a_fix_that_exists() {
    // A function's own `var` parameter is checked at each call (P38); a closure's keeps the error,
    // since a closure is only called through a value.
    let cases = [
        // The arena field is named as the struct spells it.
        (
            "struct Bucket {\n pool: Arena = Arena()\n ids: [int] = []\n}\nmain() {\n let f = (var b: Bucket) {\n  b.ids = [1]\n }\n}\n",
            "allocate it with `in b.pool { }`",
        ),
        // No arena field: say to add one.
        (
            "struct Tag {\n ids: [int] = []\n}\nmain() {\n let f = (var t: Tag) {\n  t.ids = [1]\n }\n}\n",
            "give `Tag` an `Arena` field, and build it inside `in t.<that field> { }`",
        ),
        // An element of a list parameter is named as the loop spells it.
        (
            "main() {\n let f = (var cs: list[Chunk]) {\n  for c in cs {\n   c.heights = [1]\n  }\n }\n}\n",
            "allocate it with `in c.mem { }`",
        ),
        // A `match` arm's binding is a variant: name the variant, not the enum.
        (
            "enum Msg {
 Say { text: string }
 Quit
}
main() {
 let f = (var m: Msg, n: int) {
  match m {
   Say s { s.text = \"n{n}\" }
   Quit { }
  }
 }
}
",
            "give the `Say` variant an `Arena` field, and build it inside `in s.<that field> { }`",
        ),
        // A list has no fields: its owner needs the arena.
        (
            "main() {\n let f = (var xs: list[[int]]) {\n  xs.add([1])\n }\n}\n",
            "build it in the caller, or give the owner of `xs` an `Arena` field and build it inside `in <owner>.<that field> { }`",
        ),
        // A call that stores into a closure's `var` parameter gets the same help.
        (
            "struct Bucket {\n pool: Arena = Arena()\n ids: [int] = []\n}\nfill(var b: Bucket) {\n b.ids = [1]\n}\nmain() {\n let f = (var b: Bucket) {\n  fill(var b)\n }\n}\n",
            "allocate it with `in b.pool { }`",
        ),
    ];
    for (body, expected) in cases {
        let found = helps(&format!("{CHUNK}{body}"));
        assert_eq!(found, vec![Some(expected.to_string())], "source:\n{body}");
    }
    let errs = run(&format!("{CHUNK}{}", cases[5].0)).1;
    assert_eq!(errs, ["`fill` stores memory in `b` that is in the caller's current arena, which may not live as long as `b`"]);
}

#[test]
fn a_default_that_holds_memory_is_built_in_the_current_arena() {
    let freed = "this memory is freed at the end of the `arena { }` block, but `keep` lives longer";
    let build_in_arena = "main() {\n var keep = S()\n arena {\n  keep = S()\n }\n}\n";
    // Memory from a constant (`XS`) is copied into the arena that builds the struct, like `[1, 2, 3]`.
    for defaults in [
        "let XS = [1, 2, 3]
struct S {
 xs: [int] = XS
}
",
        "struct S {
 xs: [int] = [1, 2, 3]
}
",
        "struct S {
 xs: list[int] = []
}
",
        "let BY_ID = {1: 2}
struct S {
 m: map[int, int] = BY_ID
}
",
    ] {
        assert_eq!(run(&format!("{defaults}{build_in_arena}")).1, vec![freed.to_string()], "{defaults}");
    }
    // Literals, `none` and an empty array point at no arena, so these defaults are fine anywhere.
    let fine = "let NAME = \"anon\"
struct S {
    name: string = \"anon\"
    tag: string = NAME
    nick: string? = none
    ids: [int] = []
}
main() {
    var keep = S()
    arena {
        keep = S()
    }
    print(keep)
}
";
    assert_eq!(run(fine).1, Vec::<String>::new());
}
