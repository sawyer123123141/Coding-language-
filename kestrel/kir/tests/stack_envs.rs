//! Stack environments: a closure literal that is directly a call's argument is made with its
//! captured values shallow-copied into a stack slot (`EnvKind::Stack`), and that function value
//! appears only as an argument of the call (`check_stack_envs`).

use std::path::Path;

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
    m.funcs.iter().find(|f| f.name == name).unwrap_or_else(|| panic!("no function {name}"))
}

/// The environments made in `stmts`, in order: `true` for a stack one, `false` for a heap one.
fn envs(stmts: &[Stmt]) -> Vec<(bool, Env)> {
    let mut out = Vec::new();
    visit::walk(stmts, &mut |n| match n {
        visit::Node::Expr(Expr { kind: ExprKind::MakeFn { env: EnvKind::Stack(env), .. }, .. }) => out.push((true, env.clone())),
        visit::Node::Expr(Expr { kind: ExprKind::MakeFn { env: EnvKind::Heap(env), .. }, .. }) => out.push((false, env.clone())),
        _ => {}
    });
    out
}

const SRC: &str = "struct Keep {
    fns: list[(int) -> int] = []
}

apply(f: (int) -> int, n: int) -> int { f(n) }

main() {
    let k = 3
    let s = \"ab\"
    let c = true
    var keep = Keep()
    print(apply((x) { x + k + s.len }, 1))
    print(apply(n: 2, f: (x) { x + k }))
    let g = (h: (int) -> int) -> int { h(4) }
    print(g((x) { x * k }))
    print(apply(if c { (x: int) -> int { x - k } } else { (x: int) -> int { x } }, 5))
    keep.fns.add((x) { x + k })
}
";

#[test]
fn a_literal_passed_straight_to_a_call_gets_a_stack_environment_of_plain_reads() {
    let m = lower_src(SRC);
    let made = envs(&func(&m, "main").body);
    let kinds: Vec<bool> = made.iter().map(|(stack, _)| *stack).collect();
    // `apply`'s two literals and `g`'s are stack environments; the one in the `if` and the one
    // `add` stores are heap ones. (`g` itself captures nothing, so it has no environment.)
    assert_eq!(kinds, vec![true, true, true, false, false]);
    for (stack, env) in &made {
        if *stack {
            // Shallow copies: each value is a plain read of the captured place.
            assert!(env.values.iter().all(|v| matches!(v.kind, ExprKind::Load(_))), "{env:?}");
        }
    }
    // The header holds the real copy function, the one a heap environment of the literal would.
    let (_, first) = &made[0];
    assert!(m.funcs[first.copy.0 as usize].name.starts_with("__kestrel_env_copy_"));
    assert_eq!(first.layout.offsets.len(), 2);
    assert_eq!(check_stack_envs(&m), Ok(()));
}

#[test]
fn a_labelled_call_keeps_the_literal_an_argument() {
    let m = lower_src(SRC);
    // `apply(n: 2, f: ...)` evaluates its arguments in source order through temporaries; the
    // function value stays the call's own argument.
    let mut direct = 0;
    visit::walk(&func(&m, "main").body, &mut |n| {
        if let visit::Node::Expr(Expr { kind: ExprKind::Call(_, args), .. }) = n {
            direct += args.iter().filter(|a| matches!(a, Arg::Value(Expr { kind: ExprKind::MakeFn { env: EnvKind::Stack(_), .. }, .. }))).count();
        }
    });
    assert_eq!(direct, 2);
}

#[test]
fn a_stack_environment_outside_a_call_is_refused() {
    let mut m = lower_src(SRC);
    let main = m.funcs.iter_mut().find(|f| f.name == "main").unwrap();
    let mut stray = None;
    visit::walk(&main.body, &mut |n| {
        if let visit::Node::Expr(e @ Expr { kind: ExprKind::MakeFn { env: EnvKind::Stack(_), .. }, .. }) = n {
            stray.get_or_insert_with(|| e.clone());
        }
    });
    let stray = stray.expect("a stack environment");
    let tmp = LocalIdx(main.locals.len() as u32);
    main.locals.push(Local { name: "tmp".into(), ty: KTy::Fn, kind: LocalKind::Value });
    main.body.push(Stmt::Let(tmp, stray));
    assert_eq!(check_stack_envs(&m), Err("`main` has a function value with a stack environment that isn't a call's argument".to_string()));
}

#[test]
fn every_program_test_keeps_its_stack_environments_as_call_arguments() {
    // Deeply nested test programs need the stack the compiler itself runs on (see `cli`).
    let sweep = std::thread::Builder::new().stack_size(256 << 20).spawn(sweep_programs).unwrap();
    sweep.join().unwrap();
}

fn sweep_programs() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../tests/programs");
    let mut checked = 0;
    for entry in std::fs::read_dir(&dir).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_none_or(|e| e != "kes") {
            continue;
        }
        let src = std::fs::read_to_string(&path).unwrap();
        // Lowering itself checks this in debug builds, so this also runs it in release ones.
        let src = kestrel_driver::without_bom(&src);
        let (compiled, text) = kestrel_driver::compile_unoptimized(&path.to_string_lossy(), src, kestrel_driver::DiagStyle::Short, true);
        let Some(compiled) = compiled else {
            // A program test may be one that fails to compile on purpose: none are, today.
            panic!("{}: doesn't compile:\n{text}", path.display());
        };
        if let Err(e) = check_stack_envs(&compiled.module) {
            panic!("{}: {e}", path.display());
        }
        checked += 1;
    }
    assert!(checked > 100, "only {checked} program tests found in {}", dir.display());
}
