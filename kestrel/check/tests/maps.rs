use kestrel_check::{check, Tables, Ty};
use kestrel_syntax::parse;

fn run(src: &str) -> (Option<Tables>, Vec<String>) {
    let (p, d) = parse(src);
    assert!(d.is_empty(), "parse errors: {:?}", d.iter().map(|d| &d.message).collect::<Vec<_>>());
    let (t, d) = check(&p);
    (t, d.into_iter().map(|d| d.message).collect())
}

fn local_ty(t: &Tables, name: &str) -> Ty {
    t.locals.iter().find(|l| l.name == name).unwrap_or_else(|| panic!("no local {name}")).ty
}

const PROGRAM: &str = "struct Inv {
    items: map[int, int] = {}
}

pure tally(words: [string]) -> map[string, int] {
    var m: map[string, int] = {}
    for w in words {
        m[w] = m.get(w, 0) + 1
    }
    m
}

main() {
    var prices = {\"sword\": 30, \"bow\": 12}
    prices[\"axe\"] = 5
    let has = prices.has(\"bow\")
    let gone = prices.remove(\"bow\")
    let maybe = prices[\"sword\"]
    let n = prices.len
    for name, price in prices {
        price += 1
    }
    for key in prices {
        print(key)
    }
    var inv = Inv()
    inv.items[1] = 10
    var big = map[i64, list[int]]()
    big[3] = [1, 2]
    print(has, gone, maybe, n, tally([\"a\"]), \"a\" == \"b\", big)
}
";

#[test]
fn every_map_operation_checks() {
    let (t, d) = run(PROGRAM);
    assert!(d.is_empty(), "{d:?}");
    let t = t.unwrap();
    let ty = |name| t.ty_name(local_ty(&t, name));
    assert_eq!(ty("prices"), "map[string, int]");
    assert_eq!(ty("maybe"), "int?");
    assert_eq!((ty("has"), ty("gone"), ty("n")), ("bool".into(), "bool".into(), "int".into()));
    assert_eq!((ty("name"), ty("price"), ty("key")), ("string".into(), "int".into(), "string".into()));
    assert_eq!(ty("big"), "map[i64, list[int]]");
}

#[test]
fn a_loop_value_changes_the_map_only_when_the_map_can_change() {
    let src = "main() {\n let m = {1: 2}\n for k, v in m {\n v = 3\n }\n}\n";
    let (_, d) = run(src);
    assert_eq!(d.len(), 1, "{d:?}");
    assert_eq!(d[0], "can't change `v`: it is a value in `m`, which was declared with `let`");
}
