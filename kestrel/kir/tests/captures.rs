//! Closures with captures in KIR: the heap environment a `MakeFn` builds, how the closure's code
//! reads it, the copy function in its header, and copies of function values.

use kestrel_kir::*;
use kestrel_syntax::{parse, LineIndex};

fn lower_src(src: &str, passes: &[&str]) -> Module {
    let (p, d) = parse(src);
    assert!(d.is_empty(), "parse: {:?}", d.iter().map(|d| &d.message).collect::<Vec<_>>());
    let (t, d) = kestrel_check::check(&p);
    assert!(d.is_empty(), "check: {:?}", d.iter().map(|d| &d.message).collect::<Vec<_>>());
    let mut m = lower(&p, &t.unwrap(), &LineIndex::new(src));
    for pass in passes {
        match *pass {
            "inline" => inline::inline_small(&mut m),
            "scalarize" => scalarize::scalarize(&mut m),
            "promote" => promote::promote(&mut m),
            other => panic!("no pass {other}"),
        }
    }
    m
}

fn func<'m>(m: &'m Module, name: &str) -> &'m Func {
    m.funcs.iter().find(|f| f.name == name).unwrap_or_else(|| panic!("no function {name}"))
}

fn named<'m>(m: &'m Module, prefix: &str) -> Vec<(FuncIdx, &'m Func)> {
    m.funcs.iter().enumerate().filter(|(_, f)| f.name.starts_with(prefix)).map(|(i, f)| (FuncIdx(i as u32), f)).collect()
}

/// The heap environments made in `stmts`: the code, then the environment.
fn heap_envs(stmts: &[Stmt]) -> Vec<(FuncIdx, Env)> {
    let mut out = Vec::new();
    visit::walk(stmts, &mut |n| {
        if let visit::Node::Expr(Expr { kind: ExprKind::MakeFn { code, env: EnvKind::Heap(env) }, .. }) = n {
            out.push((*code, env.clone()));
        }
    });
    out
}

fn binds(stmts: &[Stmt]) -> Vec<(LocalIdx, u32)> {
    stmts
        .iter()
        .filter_map(|s| match s {
            Stmt::BindEnv { local, offset, .. } => Some((*local, *offset)),
            _ => None,
        })
        .collect()
}

const SRC: &str = "main() {
    let name = \"ann\"
    let n = 3
    let f = () { print(name, n) }
    f()
}
";

#[test]
fn a_closure_with_captures_builds_a_heap_environment() {
    let m = lower_src(SRC, &[]);
    let envs = heap_envs(&func(&m, "main").body);
    assert_eq!(envs.len(), 1);
    let (code, env) = &envs[0];
    // The header (copy function, size), then the string at 16 and the int at 32.
    assert_eq!(env.layout, EnvLayout { offsets: vec![16, 32], size: 40 });
    assert_eq!(env.values.iter().map(|v| v.ty).collect::<Vec<_>>(), vec![KTy::Str, KTy::I32]);
    // The string is copied deep: its value is a copy of `name`.
    let mut dups = 0;
    visit::walk_expr(&env.values[0], &mut |n| {
        if let visit::Node::Stmt(Stmt::Dup(_)) = n {
            dups += 1;
        }
    });
    assert_eq!(dups, 1);
    let copy = &m.funcs[env.copy.0 as usize];
    assert!(copy.name.starts_with("__kestrel_env_copy_"), "{}", copy.name);
    assert!(m.funcs[code.0 as usize].name.starts_with("__kestrel_closure_"));
}

#[test]
fn the_closure_binds_each_capture_to_its_place_in_the_environment() {
    let m = lower_src(SRC, &[]);
    let (_, code) = named(&m, "__kestrel_closure_")[0];
    let b = binds(&code.body);
    assert_eq!(b.iter().map(|&(_, o)| o).collect::<Vec<_>>(), vec![16, 32]);
    for &(l, _) in &b {
        assert_eq!(code.locals[l.0 as usize].kind, LocalKind::RefParam);
        assert!(!code.params.contains(&l));
    }
    assert_eq!(code.locals[b[0].0 .0 as usize].ty, KTy::Str);
    // The body reads the captures through them.
    let mut read = Vec::new();
    visit::places(&code.body, &mut |p, _| read.push(p.root));
    assert!(b.iter().all(|(l, _)| read.contains(l)));
}

#[test]
fn the_copy_function_copies_the_environment_and_what_it_holds() {
    let m = lower_src(SRC, &[]);
    let (_, copy) = named(&m, "__kestrel_env_copy_")[0];
    assert_eq!(copy.params, vec![LocalIdx(0)]);
    assert_eq!((copy.locals[0].ty, copy.locals[0].kind), (KTy::I64, LocalKind::Value));
    assert_eq!(copy.ret, Some(KTy::I64));
    assert!(matches!(&copy.body[0], Stmt::Let(_, Expr { kind: ExprKind::EnvDup(_), ty: KTy::I64, .. })));
    assert_eq!(binds(&copy.body).iter().map(|&(_, o)| o).collect::<Vec<_>>(), vec![16, 32]);
    // The string gets its own bytes; the int needs nothing.
    assert_eq!(copy.body.iter().filter(|s| matches!(s, Stmt::Dup(_))).count(), 1);
    assert!(matches!(copy.body.last(), Some(Stmt::Return(Some(_)))));
}

#[test]
fn copying_a_function_value_copies_its_environment() {
    let m = lower_src("main() {\n    let n = 3\n    let f = () { n }\n    let g = f\n    print(g())\n}\n", &[]);
    let mut copies = 0;
    visit::walk(&func(&m, "main").body, &mut |n| {
        if let visit::Node::Stmt(Stmt::CopyFnEnv(_)) = n {
            copies += 1;
        }
    });
    assert_eq!(copies, 1);
    // A closure that captures a function value copies it when it is made, and its copy function
    // copies it again.
    let m = lower_src("main() {\n    let n = 3\n    let f = () { n }\n    let g = () { f() }\n    print(g())\n}\n", &[]);
    let copy = named(&m, "__kestrel_env_copy_").into_iter().map(|(_, f)| f).find(|f| binds(&f.body).len() == 1 && f.locals[2].ty == KTy::Fn);
    let copy = copy.expect("the copy function of `g`");
    assert!(copy.body.iter().any(|s| matches!(s, Stmt::CopyFnEnv(_))));
}

#[test]
fn the_passes_leave_heap_environments_alone() {
    let src = "struct S {\n    name: string\n    cb: () -> string = () { \"\" }\n}\nmain() {\n    var s = S(name: \"a\")\n    let v = vec2(1, 2)\n    s.cb = () { \"{s.name} {v.x}\" }\n    let t = s\n    print(t.cb())\n}\n";
    let m = lower_src(src, &["inline", "scalarize", "promote"]);
    assert_eq!(heap_envs(&func(&m, "main").body).len(), 1);
    let (_, code) = named(&m, "__kestrel_closure_").into_iter().find(|(_, f)| !binds(&f.body).is_empty()).expect("the closure");
    assert_eq!(binds(&code.body).len(), 2);
}
