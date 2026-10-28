//! Storage-reusing assignment: `target = value` over an existing place calls a generated
//! `__kestrel_assign_` helper that copies into the target's buffers when they have room. Places
//! that may overlap, targets whose index can write, plain data and fresh values keep today's
//! lowering.

mod range_common;

use kestrel_kir::range::Arith::Checked;
use kestrel_kir::range::CheckKind::Bounds;
use kestrel_kir::*;
use kestrel_syntax::{parse, LineIndex};
use range_common::*;

fn lower_src(src: &str) -> Module {
    let (p, d) = parse(src);
    assert!(d.is_empty(), "parse: {:?}", d.iter().map(|d| &d.message).collect::<Vec<_>>());
    let (t, d) = kestrel_check::check(&p);
    assert!(d.is_empty(), "check: {:?}", d.iter().map(|d| &d.message).collect::<Vec<_>>());
    lower(&p, &t.unwrap(), &LineIndex::new(src))
}

fn func<'m>(m: &'m Module, name: &str) -> &'m Func {
    m.funcs.iter().find(|f| f.name == name).unwrap_or_else(|| panic!("no function `{name}`"))
}

/// Every call in `name`'s body to a function whose name starts with `__kestrel_assign_`, with its
/// arguments.
fn assign_calls<'m>(m: &'m Module, name: &str) -> Vec<&'m [Arg]> {
    let mut out = Vec::new();
    visit::walk(&func(m, name).body, &mut |n| {
        if let visit::Node::Stmt(Stmt::Call(f, args, _)) = n {
            if m.funcs[f.0 as usize].name.starts_with("__kestrel_assign_") {
                out.push(&args[..]);
            }
        }
    });
    out
}

/// Every `Assign` in `name`'s body whose value is a `Seq` (today's copy: a temporary, then its
/// own copies of what it holds), with its place.
fn copying_assigns<'m>(m: &'m Module, name: &str) -> Vec<&'m Place> {
    let mut out = Vec::new();
    visit::walk(&func(m, name).body, &mut |n| {
        if let visit::Node::Stmt(Stmt::Assign(p, Expr { kind: ExprKind::Seq(..), .. })) = n {
            out.push(p);
        }
    });
    out
}

const TYPES: &str = "struct State {\n xs: list[int]\n name: string\n hp: int\n subs: list[State]\n}\n\
struct Pair {\n game: State\n ring: list[State]\n}\n\
struct Node {\n v: int\n kids: list[Node]\n}\n\
struct Game {\n xs: list[int]\n history: list[Game]\n}\n\
mk() -> State {\n State(xs: [1], name: \"s\", hp: 1, subs: [])\n}\n";

/// `TYPES` and a `main` holding `body`.
fn with_main(body: &str) -> String {
    format!("{TYPES}main() {{\n{body}\n}}\n")
}

#[test]
fn a_snapshot_assignment_calls_the_assign_helper() {
    let src = with_main(" var ring: list[State] = [mk(), mk()]\n var g = mk()\n let k = 1\n ring[k] = g\n print(ring)");
    let m = lower_src(&src);
    let calls = assign_calls(&m, "main");
    let [[Arg::Ref(dst), Arg::Ref(src)]] = &calls[..] else { panic!("{:#?}", func(&m, "main").body) };
    assert!(matches!(&dst.path[..], [Proj::Index { .. }]), "the target is `ring[k]`: {dst:?}");
    assert!(src.path.is_empty(), "the source is `g`: {src:?}");
    assert_ne!(dst.root, src.root);
    let helper = m.funcs.iter().find(|f| f.name.starts_with("__kestrel_assign_")).unwrap();
    assert!(helper.is_pure && helper.mutates_params);
    assert_eq!(helper.lines, (0, 0));
    assert!(copying_assigns(&m, "main").is_empty(), "no copy is made at the assignment itself");
}

#[test]
fn overlapping_places_keep_the_old_copy() {
    let shapes = [
        " var a = mk()\n a = a\n print(a)",
        " var t = Node(v: 1, kids: [Node(v: 2, kids: [])])\n t = t.kids[0]\n print(t)",
        " var n = Node(v: 1, kids: [Node(v: 2, kids: [])])\n n.kids[0] = n\n print(n)",
        " var ring: list[State] = [mk(), mk()]\n let i = 0\n let j = 1\n ring[i] = ring[j]\n print(ring)",
        " var g = Game(xs: [1], history: [Game(xs: [2], history: [])])\n for s in g.history {\n  g = s\n }\n print(g)",
        " var p = Pair(game: mk(), ring: [mk()])\n p.game = p.game.subs[0]\n print(p)",
        " var p = Pair(game: mk(), ring: [mk(), mk()])\n let k = 0\n let j = 1\n p.ring[k] = p.ring[j]\n print(p)",
    ];
    for body in shapes {
        let m = lower_src(&with_main(body));
        assert!(assign_calls(&m, "main").is_empty(), "{body}: {:#?}", func(&m, "main").body);
        assert_eq!(copying_assigns(&m, "main").len(), 1, "{body}: {:#?}", func(&m, "main").body);
    }
}

#[test]
fn field_disjoint_places_reuse() {
    for body in [
        " var p = Pair(game: mk(), ring: [mk(), mk()])\n let k = 0\n p.ring[k] = p.game\n print(p)",
        " var p = Pair(game: mk(), ring: [mk(), mk()])\n let j = 1\n p.game = p.ring[j]\n print(p)",
    ] {
        let m = lower_src(&with_main(body));
        let calls = assign_calls(&m, "main");
        assert_eq!(calls.len(), 1, "{body}: {:#?}", func(&m, "main").body);
        assert!(copying_assigns(&m, "main").is_empty(), "{body}");
    }
}

#[test]
fn a_target_index_that_can_write_keeps_the_old_copy() {
    let fns = "next(var k: int) -> int {\n k += 1\n k % 2\n}\npick(ring: list[State]) -> int {\n ring.len - 1\n}\n";
    for body in [
        " var ring: list[State] = [mk(), mk()]\n var k = 0\n let g = mk()\n ring[next(var k)] = g\n print(ring, k)",
        " var ring: list[State] = [mk(), mk()]\n let g = mk()\n ring[pick(ring)] = g\n print(ring)",
        " var ring: list[State] = [mk(), mk()]\n var k = 0\n let g = mk()\n ring[if k == 0 {\n  k = 1\n  0\n } else {\n  1\n }] = g\n print(ring, k)",
    ] {
        let m = lower_src(&format!("{fns}{}", with_main(body)));
        assert!(assign_calls(&m, "main").is_empty(), "{body}: {:#?}", func(&m, "main").body);
        // Today's order: one `Assign` of the copy, its index still inside the place (evaluated after
        // the value, as before), and nothing spilled ahead of it.
        let places = copying_assigns(&m, "main");
        let [place] = &places[..] else { panic!("{body}: {:#?}", func(&m, "main").body) };
        let Some(Proj::Index { at, .. }) = place.path.first() else { panic!("{place:?}") };
        assert!(!matches!(at.kind, ExprKind::Load(_)), "{body}: the index was not spilled: {at:?}");
    }
}

#[test]
fn plain_data_needs_no_helper() {
    let src = "struct R {\n seed: i64\n}\nstruct Ball {\n pos: vec2\n vel: vec2\n hp: int\n rng: R\n}\n\
main() {\n var ring: list[Ball] = [Ball(pos: vec2(0.0, 0.0), vel: vec2(1.0, 1.0), hp: 1, rng: R(seed: 1))]\n let g = ring[0]\n let k = 0\n ring[k] = g\n print(ring)\n}\n";
    let m = lower_src(src);
    assert!(assign_calls(&m, "main").is_empty());
    assert!(!m.funcs.iter().any(|f| f.name.starts_with("__kestrel_assign_")));
    let mut plain = 0;
    visit::walk(&func(&m, "main").body, &mut |n| {
        if let visit::Node::Stmt(Stmt::Assign(p, Expr { kind: ExprKind::Load(_), .. })) = n {
            if !p.path.is_empty() {
                plain += 1;
            }
        }
    });
    assert_eq!(plain, 1, "{:#?}", func(&m, "main").body);
}

#[test]
fn fresh_values_and_lets_are_unchanged() {
    for body in [
        " let y = mk()\n let x = y\n print(x)",
        " var g = mk()\n g = mk()\n print(g)",
        " var g = mk()\n g = State(xs: [2], name: \"t\", hp: 2, subs: [])\n print(g)",
        " var g = mk()\n let t = \"u\"\n g.name += t\n print(g)",
    ] {
        let m = lower_src(&with_main(body));
        assert!(assign_calls(&m, "main").is_empty(), "{body}: {:#?}", func(&m, "main").body);
        assert!(!m.funcs.iter().any(|f| f.name.starts_with("__kestrel_assign_")), "{body}");
    }
}

#[test]
fn a_length_fact_dies_at_the_assignment() {
    let src = "f(var xs: list[int], ys: list[int]) -> int {\n @assume(xs.len == 4)\n xs = ys\n xs[3]\n}";
    let (m, a) = analysed(src, Checked);
    let left: Vec<u32> = left_of(&m, &a, "f", Bounds).iter().map(|c| line(c.line)).collect();
    assert_eq!(left, [4]);
    // The assignment reuses (the helper is inlined, or called).
    let lowered = optimized(src);
    let mut reuses = false;
    visit::walk(&func(&lowered, "f").body, &mut |n| match n {
        visit::Node::Stmt(Stmt::CopyItems(..)) => reuses = true,
        visit::Node::Stmt(Stmt::Call(g, ..)) if lowered.funcs[g.0 as usize].name.starts_with("__kestrel_assign_") => reuses = true,
        _ => {}
    });
    assert!(reuses, "{:#?}", func(&lowered, "f").body);
}

#[test]
fn a_loop_over_the_target_reindexes() {
    let src = with_main(" var g = mk()\n let h = mk()\n for x in g.xs {\n  g = h\n  print(x)\n }");
    let m = lower_src(&src);
    assert_eq!(assign_calls(&m, "main").len(), 1, "{:#?}", func(&m, "main").body);
    let mut binds = 0;
    visit::walk(&func(&m, "main").body, &mut |n| {
        if let visit::Node::Stmt(Stmt::Bind(..)) = n {
            binds += 1;
        }
    });
    assert_eq!(binds, 0, "the element pointer isn't used: {:#?}", func(&m, "main").body);
}
