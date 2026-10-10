use kestrel_kir::*;
use kestrel_syntax::{parse, LineIndex};

fn lower_src(src: &str) -> Module {
    let (p, d) = parse(src);
    assert!(d.is_empty(), "parse: {:?}", d.iter().map(|d| &d.message).collect::<Vec<_>>());
    let (t, d) = kestrel_check::check(&p);
    assert!(d.is_empty(), "check: {:?}", d.iter().map(|d| &d.message).collect::<Vec<_>>());
    lower(&p, &t.unwrap(), &LineIndex::new(src))
}

fn func<'m>(m: &'m Module, name: &str) -> &'m Func {
    m.funcs.iter().find(|f| f.name == name).unwrap()
}

const SRC: &str = "struct C {
    x: int
}

main() {
    var cs = list[C]()
    cs.add(C(x: 1))
    let seed = 7
    par for c in cs {
        c.x += seed
    }
    print(cs)
}
";

#[test]
fn the_body_becomes_a_function_over_a_range() {
    let m = lower_src(SRC);
    let main = func(&m, "main");
    let Some(Stmt::ParFor { func: fi, args, count }) = main.body.iter().find(|s| matches!(s, Stmt::ParFor { .. })) else {
        panic!("{:#?}", main.body)
    };
    assert_eq!(m.par_bodies, vec![*fi]);
    assert!(matches!(count.kind, ExprKind::Len(_)));
    // `cs` and `seed`, by pointer.
    assert_eq!(args.len(), 2);
    assert!(args.iter().all(|a| matches!(a, Arg::Ref(_))));
    let body = &m.funcs[fi.0 as usize];
    let kinds: Vec<(LocalKind, KTy)> = body.params.iter().map(|p| (body.locals[p.0 as usize].kind, body.locals[p.0 as usize].ty)).collect();
    assert_eq!(kinds.len(), 4);
    assert!(kinds[..2].iter().all(|(k, _)| *k == LocalKind::RefParam));
    assert_eq!(&kinds[2..], &[(LocalKind::Value, KTy::I64), (LocalKind::Value, KTy::I64)]);
    assert!(matches!(&body.body[..], [Stmt::ForRange { .. }]), "{:#?}", body.body);
}

#[test]
fn a_captured_scalar_gets_an_address() {
    let m = lower_src(SRC);
    let main = func(&m, "main");
    let seed = main.locals.iter().find(|l| l.name == "seed").unwrap();
    assert_eq!(seed.kind, LocalKind::Addressed);
}

#[test]
fn nested_par_for_bodies_are_outlined_too() {
    let src = "struct C {\n xs: list[int] = list[int]()\n}\nmain() {\n var cs = list[C]()\n par for c in cs {\n par for x in c.xs {\n x += 1\n }\n }\n}\n";
    let m = lower_src(src);
    assert_eq!(m.par_bodies.len(), 2);
}
