//! Captures: what a closure literal copies from the code around it, as paths, and the rules
//! about changing, owning and storing them.

use kestrel_check::{check, ClosureInfo, Tables};
use kestrel_syntax::parse;

fn run(src: &str) -> (Option<Tables>, Vec<String>) {
    let (p, d) = parse(src);
    assert!(d.is_empty(), "parse errors: {:?}", d.iter().map(|d| &d.message).collect::<Vec<_>>());
    let (t, d) = check(&p);
    (t, d.into_iter().filter(|d| d.is_error()).map(|d| d.message).collect())
}

fn ok(src: &str) -> Tables {
    let (t, errs) = run(src);
    assert!(errs.is_empty(), "unexpected errors: {errs:?}");
    t.expect("tables")
}

fn errors(src: &str) -> Vec<String> {
    run(src).1
}

/// The closures, in the order of their ids (their order in the source).
fn closures(t: &Tables) -> Vec<&ClosureInfo> {
    let mut ids: Vec<_> = t.closures.keys().copied().collect();
    ids.sort();
    ids.iter().map(|id| &t.closures[id]).collect()
}

/// Each capture of `c` as written (`p.name`), and as the place it reads (`bs[_].x` for a loop item).
fn captured(t: &Tables, c: &ClosureInfo) -> Vec<(String, String)> {
    c.captures
        .iter()
        .map(|cap| {
            let mut written = t.local(cap.root).name.clone();
            let mut ty = t.local(cap.root).ty;
            for &f in &cap.path {
                let fields = match ty {
                    kestrel_check::Ty::Struct(s) => &t.strukt(s).fields,
                    kestrel_check::Ty::Variant(e, v) => &t.enm(e).variants[v as usize].fields,
                    other => panic!("a capture path steps into {other:?}"),
                };
                written.push('.');
                written.push_str(&fields[f as usize].name);
                ty = fields[f as usize].ty;
            }
            assert_eq!(ty, cap.ty, "the type of {written}");
            let mut source = t.local(cap.source_root).name.clone();
            for step in &cap.source_path {
                match step {
                    Some(f) => {
                        source.push('.');
                        source.push_str(f);
                    }
                    None => source.push_str("[_]"),
                }
            }
            (written, source)
        })
        .collect()
}

fn names(t: &Tables, c: &ClosureInfo) -> Vec<String> {
    captured(t, c).into_iter().map(|(w, _)| w).collect()
}

#[test]
fn a_capture_follows_fields_and_stops_at_an_index_a_property_or_a_method() {
    let t = ok(concat!(
        "struct P {\n    name: string\n    pos: vec2\n}\n",
        "main() {\n",
        "    let p = P(name: \"a\", pos: vec2(1, 2))\n",
        "    let xs: list[int] = [1]\n",
        "    let i = 0\n",
        "    let f = () { p.name.len + xs[i] + xs.len }\n",
        "    let g = () { p.pos.x + p.pos.distance_to(vec2(0, 0)) }\n",
        "    let h = () { p.pos.x + p.pos.y }\n",
        "    let w = () { p }\n",
        "}\n",
    ));
    let cs = closures(&t);
    assert_eq!(names(&t, cs[0]), vec!["p.name", "xs", "i"]);
    // `p.pos` covers `p.pos.x`.
    assert_eq!(names(&t, cs[1]), vec!["p.pos"]);
    assert_eq!(names(&t, cs[2]), vec!["p.pos.x", "p.pos.y"]);
    assert_eq!(names(&t, cs[3]), vec!["p"]);
}

#[test]
fn capture_reads_say_which_capture_each_read_is_and_what_is_left() {
    let t = ok("main() {\n    let v = vec2(1, 2)\n    let g = () { v.x + v.length() }\n}\n");
    let (&id, info) = t.closures.iter().next().expect("one closure");
    assert_eq!(names(&t, info), vec!["v"]);
    let mut reads: Vec<_> = t.capture_reads.values().cloned().collect();
    reads.sort_by(|a, b| a.2.cmp(&b.2));
    // `v` (the receiver of `length`) is the capture itself; `v.x` is its field 0.
    assert_eq!(reads, vec![(id, 0, vec![]), (id, 0, vec![0])]);
}

#[test]
fn a_closure_inside_a_closure_brings_its_captures_out_to_the_outer_one() {
    let t = ok("main() {\n    let n = 1\n    let k = 2\n    let f = (x: int) {\n        let g = () { x + n }\n        g() + k\n    }\n}\n");
    let cs = closures(&t);
    // (The inner literal's id comes first.)
    assert_eq!(names(&t, cs[0]), vec!["x", "n"]);
    assert_eq!(names(&t, cs[1]), vec!["n", "k"]);
}

#[test]
fn a_view_is_captured_as_written_and_reads_its_source() {
    let t = ok(concat!(
        "struct B {\n    x: int\n}\n",
        "enum K {\n    Plain\n    Coin { value: int }\n}\n",
        "main() {\n",
        "    let bs: list[B] = [B(x: 1)]\n",
        "    for b in bs {\n        let f = () { b.x }\n    }\n",
        "    let k = K.Coin(value: 3)\n",
        "    match k {\n        Coin c { let g = () { c.value } }\n        else { }\n    }\n",
        "    let maybe: string? = \"a\"\n",
        "    let got = maybe else { return }\n",
        "    let h = () { got }\n",
        "}\n",
    ));
    let cs = closures(&t);
    assert_eq!(captured(&t, cs[0]), vec![("b.x".to_string(), "bs[_].x".to_string())]);
    assert_eq!(captured(&t, cs[1]), vec![("c.value".to_string(), "k.value".to_string())]);
    assert_eq!(captured(&t, cs[2]), vec![("got".to_string(), "maybe".to_string())]);
}

#[test]
fn a_function_field_called_through_a_capture_captures_the_field() {
    let t = ok("struct B {\n    on: (int) -> int = (n) { n }\n}\nmain() {\n    let b = B()\n    let f = () { b.on(1) }\n    let g = (x: int) { x }\n    let h = () { g(2) }\n}\n");
    let cs = closures(&t);
    assert_eq!(names(&t, cs[1]), vec!["b.on"]);
    assert_eq!(names(&t, cs[3]), vec!["g"]);
}

#[test]
fn captures_cannot_change() {
    let msg = |name: &str| format!("`{name}` is captured by this closure, so it can't change here; return a value instead, or use a `for` loop");
    assert_eq!(errors("main() {\n    var n = 1\n    let f = () { n = 2 }\n}\n"), vec![msg("n")]);
    assert_eq!(errors("bump(var x: int) { }\nmain() {\n    var n = 1\n    let f = () { bump(var n) }\n}\n"), vec![msg("n")]);
    assert_eq!(errors("struct S {\n    xs: list[int]\n}\nmain() {\n    var s = S(xs: [])\n    let f = () { s.xs.add(1) }\n}\n"), vec![msg("s")]);
    // A closure inside a closure can't change the outer closure's parameter either.
    assert_eq!(errors("main() {\n    let f = (var x: int) {\n        let g = () { x += 1 }\n    }\n}\n"), vec![msg("x")]);
    // Its own locals and `var` parameters can change.
    ok("main() {\n    let n = 1\n    let f = (var x: int) {\n        var m = n\n        m += 1\n        x += m\n    }\n}\n");
}

#[test]
fn owning_values_cannot_be_captured_but_their_plain_fields_can() {
    let src = "struct H {\n    mem: Arena = Arena()\n    size: int = 0\n\n    total(self) -> int { self.size }\n}\nmain() {\n    let h = H()\n    let f = () { h.size }\n    let g = () { h.total() }\n}\n";
    assert_eq!(errors(src), vec!["a closure can't capture `h`: it owns an `Arena`; pass it as a parameter instead"]);
}

#[test]
fn a_stored_closure_cannot_capture_a_whole_value_holding_a_function() {
    let src = "struct B {\n    name: string\n    cb: () -> string = () { \"\" }\n\n    go(self) -> string { self.name }\n}\nmain() {\n    var b = B(name: \"a\")\n    b.cb = () { b.go() }\n    b.cb = () { b.name }\n    let f = () { b.cb }\n}\n";
    assert_eq!(errors(src), vec!["this closure would copy all of `b`, including its function fields; copy what it needs into a local first"]);
    // A struct holding a function inside a list is refused too: `self` would nest copies.
    let src = "struct H {
    handlers: list[(int)]

    hook(self) -> int {
        let f = () { self.handlers.len }
        let g = () { self }
        f()
    }
}
main() { }
";
    assert_eq!(errors(src), vec!["this closure would copy all of `self`, including its function fields; copy what it needs into a local first"]);
    // A function value, or a list, array, map or optional of them, may be captured whole.
    ok("main() {
    var hs: list[(int) -> int] = [(x) { x }]
    let a: [(int) -> int] = [(x) { x }]
    var m = map[string, (int) -> int]()
    let f = hs[0]
    let g = () { hs[0](1) + a[0](2) + m.get(\"k\", f)(3) + f(4) }
}
");
}

#[test]
fn a_closure_with_captures_lives_in_the_current_arena() {
    let freed = "this memory is freed at the end of the `arena { }` block, but `f` lives longer";
    assert_eq!(errors("main() {\n    let k = 1\n    var f = (x: int) { x }\n    arena {\n        f = (x: int) { x + k }\n    }\n}\n"), vec![freed]);
    assert_eq!(
        errors("make(k: int) -> (int) -> int {\n    arena {\n        return (x) { x + k }\n    }\n}\nmain() { }\n"),
        vec!["can't return memory from an `arena { }` block: it's freed at the block's end"]
    );
    // Returned from a function, it lives in the caller's arena.
    ok("make(k: int) -> (int) -> int {\n    (x) { x + k }\n}\nmain() { }\n");
    // Inside the body, a capture is like a parameter: returning it copies it.
    ok("main() {\n    let xs: list[int] = [1]\n    let g = () { xs }\n    arena {\n        let h = () { xs }\n        let ys = h()\n    }\n}\n");
}

#[test]
fn a_par_for_body_reads_what_its_closures_capture() {
    let src = "struct W {\n    score: int = 1\n    chunks: list[int] = [1]\n\n    pure total(self) -> int { self.score }\n}\nmain() {\n    var w = W()\n    par for c in w.chunks {\n        let f = () { w.score }\n        let g = () { w.total() }\n        c = c + f() + g()\n    }\n}\n";
    assert_eq!(errors(src), vec!["this reads `w.chunks`, whose items other iterations are changing; use the loop's item (`c`)"]);
}
