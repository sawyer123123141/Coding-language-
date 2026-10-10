use kestrel_check::check;
use kestrel_syntax::parse;

fn errors(src: &str) -> Vec<String> {
    let (p, d) = parse(src);
    assert!(d.is_empty(), "parse errors: {:?}", d.iter().map(|d| &d.message).collect::<Vec<_>>());
    check(&p).1.into_iter().map(|d| d.message).collect()
}

/// Everything a `par for` body may do.
const ALLOWED: &str = "enum Block {
    Air
    Stone { hardness: int }
}

struct Chunk {
    x: int
    mem: Arena = Arena()
    heights: [int] = []
    block: Block
    faces: int = 0

    pure build(var self, seed: int) {
        in self.mem {
            self.heights = array[int](4)
        }
        self.faces = self.x + seed
    }
}

pure bump(var n: int, by: int) {
    n += by
}

pure count(xs: [int]) -> int {
    var n = 0
    for x in xs { n += x }
    n
}

main() {
    var world = list[Chunk]()
    world.add(Chunk(x: 1, block: Block.Air))
    let seed = 7
    var other = [1, 2, 3]
    par for i, c in world {
        c.build(seed)
        var local = list[int]()
        local.add(i + seed + other[0])
        bump(var c.faces, local.len)
        for h in c.heights {
            h += 1
        }
        match c.block {
            Stone s { s.hardness = 2 }
            else {}
        }
        arena {
            let tmp = array[int](10)
            c.x += count(tmp)
        }
        in c.mem {
            c.heights = array[int](2)
        }
        if c.x > 100 { continue }
        for j from 0 to 3 {
            if j == 2 { break }
        }
    }
    print(world.len)
}
";

#[test]
fn everything_a_par_for_body_may_do_checks() {
    let e = errors(ALLOWED);
    assert!(e.is_empty(), "{e:?}");
}

#[test]
fn reading_other_fields_of_the_collections_owner_is_fine() {
    let src = "struct C {\n x: int\n}\nstruct W {\n cs: list[C] = list[C]()\n scale: int = 2\n}\nmain() {\n var w = W()\n par for c in w.cs {\n c.x = w.scale\n }\n}\n";
    let e = errors(src);
    assert!(e.is_empty(), "{e:?}");
}

#[test]
fn match_bindings_of_shared_places_are_shared() {
    let src = "enum B {\n A { n: int }\n}\nstruct C {\n x: int\n}\nmain() {\n var shared = B.A(n: 1)\n var cs = list[C]()\n par for c in cs {\n match shared {\n A a { a.n = c.x }\n }\n }\n}\n";
    let e = errors(src);
    assert_eq!(e.len(), 1, "{e:?}");
    assert!(e[0].contains("can't change `a.n`"), "{e:?}");
}

#[test]
fn nested_par_for_bodies_own_only_their_own_items() {
    let src = "struct C {\n xs: list[int] = list[int]()\n n: int = 0\n}\nmain() {\n var cs = list[C]()\n par for c in cs {\n par for x in c.xs {\n x += 1\n c.n += 1\n }\n }\n}\n";
    let e = errors(src);
    assert_eq!(e.len(), 1, "{e:?}");
    assert!(e[0].contains("can't change `c.n`"), "{e:?}");
}
