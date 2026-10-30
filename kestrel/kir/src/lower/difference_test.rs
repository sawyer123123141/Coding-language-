use kestrel_syntax::{parse, LineIndex};

use crate::ir::*;
use crate::visit;

/// The module of a program whose `main` calls `first_difference` on two `Game`s.
fn module() -> Module {
    let src = "struct Game {
    ball: vec2
    xs: list[int]
    seen: map[string, int]
}
main() {
    let g = Game(ball: vec2(1, 2), xs: [1], seen: {\"a\": 1})
    print(first_difference(g, g))
}
";
    let (p, d) = parse(src);
    assert!(d.is_empty(), "parse errors: {:?}", d.iter().map(|d| &d.message).collect::<Vec<_>>());
    let (t, d) = kestrel_check::check(&p);
    assert!(d.is_empty(), "check errors: {:?}", d.iter().map(|d| &d.message).collect::<Vec<_>>());
    crate::lower(&p, &t.unwrap(), &LineIndex::new(src))
}

fn named<'m>(m: &'m Module, prefix: &str) -> Vec<&'m Func> {
    m.funcs.iter().filter(|f| f.name.starts_with(prefix)).collect()
}

/// The `Format`s in `stmts` that run whatever the values are: outside every `if`.
fn unconditional_formats(stmts: &[Stmt]) -> usize {
    stmts
        .iter()
        .map(|s| match s {
            Stmt::Let(_, e) | Stmt::Assign(_, e) => usize::from(matches!(e.kind, ExprKind::Format(_))),
            Stmt::While { body, .. } | Stmt::ForRange { body, .. } => unconditional_formats(body),
            _ => 0,
        })
        .sum()
}

#[test]
fn each_collection_has_its_own_walk_and_every_walk_is_pure() {
    let m = module();
    let walks = named(&m, "__kestrel_diff_");
    // `Game`, `list[int]` and `map[string, int]`.
    assert_eq!(walks.len(), 3, "{:?}", walks.iter().map(|f| &f.name).collect::<Vec<_>>());
    for f in &walks {
        assert!(f.is_pure && !f.io, "{} is pure", f.name);
        assert_eq!((f.params.len(), f.ret), (3, Some(KTy::Bool)), "{}", f.name);
        assert_eq!(f.locals[2].kind, LocalKind::RefParam, "{}: `out` is a `var` parameter", f.name);
    }
    let whole = named(&m, "__kestrel_first_difference_");
    assert_eq!(whole.len(), 1);
    assert!(whole[0].is_pure && matches!(whole[0].ret, Some(KTy::Optional(_))));
}

#[test]
fn the_text_is_built_only_once_a_difference_is_found() {
    let m = module();
    for f in named(&m, "__kestrel_diff_") {
        assert_eq!(unconditional_formats(&f.body), 0, "{} formats text before finding a difference", f.name);
    }
}

#[test]
fn floats_compare_by_their_bits() {
    let m = module();
    let mut bits = 0;
    for f in named(&m, "__kestrel_diff_") {
        visit::walk(&f.body, &mut |n| {
            if let visit::Node::Expr(Expr { kind: ExprKind::Hash(HashOp::Bits, _), .. }) = n {
                bits += 1;
            }
        });
    }
    // `ball.x` and `ball.y`: each side in the comparison and in the text of the bits.
    assert_eq!(bits, 8);
}
