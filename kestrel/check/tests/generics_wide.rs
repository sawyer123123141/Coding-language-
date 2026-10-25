//! The size cap at interning (P42): plain code that doubles a type at each line, or a method
//! chain that does, stops at 1,000 parts with one error at the expression, quickly, and builds
//! no oversized name.

use std::time::{Duration, Instant};

use kestrel_check::{check_keeping_tables, Tables};
use kestrel_syntax::parse;

const PAIR: &str = "struct Pair[A, B] {\n    a: A\n    b: B\n}\n";
const WIDE: &str = "making copies of `Pair` doesn't end: each use makes a bigger `Pair` (stopped after a type grew to 1,000 parts)";

/// The errors as "line: message", the tables, and how long the check took.
fn run(src: &str) -> (Vec<String>, Tables, Duration) {
    let (p, d) = parse(src);
    assert!(d.is_empty(), "parse errors: {:?}", d.iter().map(|d| &d.message).collect::<Vec<_>>());
    let start = Instant::now();
    let (t, d) = check_keeping_tables(&p, true);
    let took = start.elapsed();
    let line = |offset: u32| src[..offset as usize].matches('\n').count() + 1;
    let errs = d.into_iter().filter(|d| d.is_error()).map(|d| format!("{}: {}", line(d.span.start), d.message)).collect();
    (errs, t, took)
}

fn no_big_names(t: &Tables) {
    assert!(t.structs.iter().all(|s| s.name.len() < 20_000), "an oversized name was built");
    assert!(t.fns.iter().all(|f| f.name.len() < 20_000), "an oversized name was built");
}

#[test]
fn plain_code_that_doubles_a_type_stops_at_the_cap() {
    // 22 lines took 6 seconds and 230 MB before the cap (the names doubled with each line); the
    // language server checks on every edit. `x9`'s type list is the first past 1,000 parts.
    let mut src = format!("{PAIR}main() {{\n    let x0 = 1\n");
    for i in 1..=22 {
        src.push_str(&format!("    let x{i} = Pair(a: x{}, b: x{})\n", i - 1, i - 1));
    }
    src.push_str("    print(x22.a)\n}\n");
    let (errs, t, took) = run(&src);
    assert_eq!(errs, [format!("15: {WIDE}")]);
    assert!(took < Duration::from_secs(3), "the check took {took:?}");
    no_big_names(&t);
}

#[test]
fn a_doubling_method_chain_stops_at_the_cap() {
    // The same growth through each call's signature, with nothing to work out (`subst_call_sig`):
    // the ninth `dup` interns the first `Pair` past 1,000 parts.
    let src = format!(
        "{PAIR}struct Box[T] {{\n    v: T\n    dup(self) -> Box[Pair[T, T]] {{ Box[Pair[T, T]](v: Pair(a: self.v, b: self.v)) }}\n}}\nmain() {{\n    let b = Box(v: 1){}\n}}\n",
        ".dup()".repeat(22)
    );
    let (errs, t, took) = run(&src);
    assert_eq!(errs, [format!("10: {WIDE}")]);
    assert!(took < Duration::from_secs(3), "the check took {took:?}");
    no_big_names(&t);
}

#[test]
fn a_copy_whose_signature_passes_the_cap_is_named() {
    // `g[x8]` asks for `f[x8]`, whose parameter `Pair[x8, x8]?` would pass 1,000 parts: the walk
    // blames `f`, naming the copy (it used to say "`Box` needs `Box`", P41).
    let mut src = format!("{PAIR}pure f[T](x: T, y: Pair[T, T]?) -> int {{ 1 }}\npure g[T](x: T) -> int {{ f(x, none) }}\nmain() {{\n    let x0 = 1\n");
    for i in 1..=8 {
        src.push_str(&format!("    let x{i} = Pair(a: x{}, b: x{})\n", i - 1, i - 1));
    }
    src.push_str("    print(g(x8))\n}\n");
    let (errs, t, _) = run(&src);
    assert_eq!(errs.len(), 1, "{errs:?}");
    assert!(errs[0].starts_with("5: making copies of `f` doesn't end: `f[Pair[Pair["), "{}", errs[0]);
    assert!(errs[0].ends_with("]]]` needs a bigger one, and so on (stopped after a type grew to 1,000 parts)"), "{}", errs[0]);
    no_big_names(&t);
}
