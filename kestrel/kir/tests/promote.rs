use kestrel_kir::*;
use kestrel_syntax::{parse, LineIndex};

fn optimized(src: &str) -> Module {
    let (p, d) = parse(src);
    assert!(d.is_empty(), "parse: {:?}", d.iter().map(|d| &d.message).collect::<Vec<_>>());
    let (t, d) = kestrel_check::check(&p);
    assert!(d.is_empty(), "check: {:?}", d.iter().map(|d| &d.message).collect::<Vec<_>>());
    let mut m = lower(&p, &t.unwrap(), &LineIndex::new(src));
    optimize(&mut m);
    m
}

fn func<'m>(m: &'m Module, name: &str) -> &'m Func {
    m.funcs.iter().find(|f| f.name == name).unwrap()
}

fn local(f: &Func, name: &str) -> LocalIdx {
    LocalIdx(f.locals.iter().position(|l| l.name == name).unwrap() as u32)
}

/// How many places in `stmts` are rooted at `root`: (read, written).
fn uses(stmts: &[Stmt], root: LocalIdx) -> (usize, usize) {
    let (mut read, mut written) = (0, 0);
    visit::places(stmts, &mut |p, w| {
        if p.root == root {
            if w {
                written += 1;
            } else {
                read += 1;
            }
        }
    });
    (read, written)
}

const STEP: &str = "struct Particle {\n pos: vec2\n vel: vec2\n pure step(var self, dt: float, target: vec2) {\n  self.vel += (target - self.pos).normalized() * 900.0 * dt\n  self.pos += self.vel * dt\n  if self.pos.x < 0 {\n   self.pos.x = 0\n   self.vel.x = abs(self.vel.x) * 0.7\n  }\n }\n}\nrun(var ps: list[Particle], dt: float, target: vec2) {\n par for p in ps {\n  p.step(dt, target)\n }\n}\nmain() {}\n";

#[test]
fn a_par_for_item_is_loaded_once_and_written_back_once_per_pass() {
    let m = optimized(STEP);
    let body = &m.funcs[m.par_bodies[0].0 as usize];
    // Four fields loaded after the `Bind`, and four written back at the end of the pass.
    assert_eq!(uses(&body.body, local(body, "p_ref")), (4, 4), "{:#?}", body.body);
}

/// The statements of `f` before its first loop.
fn before_loop(f: &Func) -> Vec<Stmt> {
    f.body.iter().take_while(|s| !matches!(s, Stmt::ForRange { .. } | Stmt::While { .. })).cloned().collect()
}

#[test]
fn captured_values_are_loaded_once_per_par_for_chunk() {
    let m = optimized(STEP);
    let body = &m.funcs[m.par_bodies[0].0 as usize];
    // `dt`, and `target`'s two fields, are loaded before the loop over the chunk.
    assert_eq!(uses(&before_loop(body), local(body, "dt")), (1, 0), "{:#?}", body.body);
    assert_eq!(uses(&before_loop(body), local(body, "target")), (2, 0));
}

#[test]
fn var_self_is_written_back_before_each_return() {
    let src = "struct C {\n n: int\n m: int\n count(var self, limit: int) -> int {\n  while self.n < limit {\n   self.n += 1\n   if self.n == 5 {\n    return 5\n   }\n  }\n  self.m = self.n\n  self.n\n }\n}\nmain() {}\n";
    let m = optimized(src);
    let f = func(&m, "C.count");
    // Both fields loaded at the start; both written back before the early `return`, before the
    // final one, and at the end of the body (dead, after the final `return`).
    assert_eq!(uses(&f.body, f.params[0]), (2, 6), "{:#?}", f.body);
}

#[test]
fn a_loop_that_reads_its_list_keeps_the_item_in_memory() {
    let src = "struct P {\n x: float\n}\nmain() {\n var ps = [P(x: 1), P(x: 2)]\n for i, p in ps {\n  p.x = ps[0].x + i\n }\n print(ps)\n}\n";
    let m = optimized(src);
    let main = func(&m, "main");
    assert_eq!(uses(&main.body, local(main, "p_ref")), (0, 1), "{:#?}", main.body);
}

#[test]
fn a_loop_element_whose_list_is_freed_before_a_return_stays_in_memory() {
    // The `return` frees the arena that holds the list, right before it returns. A write-back put
    // before the `return` would store into freed memory.
    let src = "struct P {\n x: float\n}\nfind(n: int) -> float {\n arena {\n  var a = array[P](n)\n  for p in a {\n   p.x += 1\n   if p.x > 0 {\n    return p.x\n   }\n  }\n }\n 0.0\n}\nmain() {}\n";
    let m = optimized(src);
    let f = func(&m, "find");
    assert!(!f.locals.iter().any(|l| l.name.starts_with("p_ref.")), "{:#?}", f.locals);
    // `p.x += 1` stores to memory, and the other two uses read from it.
    assert_eq!(uses(&f.body, local(f, "p_ref")), (3, 1), "{:#?}", f.body);
}

#[test]
fn a_loop_element_returned_from_with_an_owning_local_to_drop_stays_in_memory() {
    // Dropping `keep` is a call that takes it by pointer, right before the `return`.
    let src = "struct P {\n x: float\n}\nstruct Chunk {\n n: int\n mem: Arena = Arena()\n}\nfind(var ps: list[P]) -> float {\n var keep = list[Chunk]()\n for p in ps {\n  p.x += 1\n  if p.x > 0 {\n   return p.x + keep.len\n  }\n }\n 0.0\n}\nmain() {}\n";
    let m = optimized(src);
    let f = func(&m, "find");
    assert!(!f.locals.iter().any(|l| l.name.starts_with("p_ref.")), "{:#?}", f.locals);
}

#[test]
fn a_loop_element_with_a_return_and_nothing_to_free_is_still_promoted() {
    let src = "struct P {\n x: float\n}\nfind(var ps: list[P]) -> float {\n for p in ps {\n  p.x += 1\n  if p.x > 0 {\n   return p.x\n  }\n }\n 0.0\n}\nmain() {}\n";
    let m = optimized(src);
    let f = func(&m, "find");
    assert!(f.locals.iter().any(|l| l.name == "p_ref.x"), "{:#?}", f.locals);
}

#[test]
fn a_pointer_parameter_is_promoted_even_when_it_returns_after_freeing() {
    // `self` belongs to the caller, so nothing the method frees can be what it points into.
    let src = "struct C {\n n: int\n f(var self, k: int) -> int {\n  self.n += k\n  arena {\n   var a = array[int](k)\n   if self.n > 3 {\n    return a.len\n   }\n  }\n  self.n\n }\n}\nmain() {}\n";
    let m = optimized(src);
    assert!(func(&m, "C.f").locals.iter().any(|l| l.name == "self.n"), "{:#?}", func(&m, "C.f").locals);
}

#[test]
fn var_self_passed_on_as_var_stays_in_memory() {
    // Both methods return early, so neither is inlined, and `twice` passes `self` on as `var`.
    let src = "struct C {\n n: int\n inc(var self) {\n  if self.n > 9 {\n   return\n  }\n  self.n += 1\n }\n twice(var self) {\n  if self.n < 0 {\n   return\n  }\n  self.inc()\n  self.n += 1\n  self.inc()\n }\n}\nmain() {}\n";
    let m = optimized(src);
    assert!(!func(&m, "C.twice").locals.iter().any(|l| l.name.starts_with("self.")));
    assert!(func(&m, "C.inc").locals.iter().any(|l| l.name == "self.n"));
}

#[test]
fn a_read_only_struct_parameter_is_loaded_once_and_never_written() {
    let src = "pure spread(a: vec2, b: vec2) -> float {\n var t = 0.0\n for i from 0 to 4 {\n  t += a.x * i + a.y - b.x * b.y\n }\n t\n}\nmain() {}\n";
    let m = optimized(src);
    let f = func(&m, "spread");
    // Each field is loaded once, before the loop, and nothing is written back.
    assert_eq!(uses(&before_loop(f), f.params[0]), (2, 0), "{:#?}", f.body);
    assert_eq!(uses(&before_loop(f), f.params[1]), (2, 0));
    assert_eq!(uses(&f.body, f.params[0]), (2, 0));
    assert_eq!(uses(&f.body, f.params[1]), (2, 0));
}
