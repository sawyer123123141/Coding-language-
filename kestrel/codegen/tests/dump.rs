mod common;

use kestrel_codegen::{dump, Options};

fn release() -> Options {
    Options { checked_arith: false, ..Options::default() }
}

#[test]
fn dump_shows_the_ir_and_the_machine_code() {
    let m = common::optimized("pure sq(x: int) -> int { x * x }\nmain() { print(sq(3)) }\n");
    let code = dump(&m, release(), "sq").unwrap();
    assert!(code.contains("function") && code.contains("imul") && code.contains("ret"), "{code}");
}

#[test]
fn dump_names_a_missing_function() {
    let m = common::optimized("main() {}\n");
    assert_eq!(dump(&m, release(), "nope").unwrap_err(), "no function named `nope`");
}

/// The particle demo's step, without the window.
const STEP: &str = "let PULL = 900.0\nlet BOUNCE = 0.7\nstruct Particle {\n pos: vec2\n vel: vec2\n pure step(var self, dt: float, target: vec2, keep: float) {\n  self.vel += (target - self.pos).normalized() * PULL * dt\n  self.vel *= keep\n  self.pos += self.vel * dt\n  if self.pos.x < 0 {\n   self.pos.x = 0\n   self.vel.x = abs(self.vel.x) * BOUNCE\n  } else if self.pos.x > 1280 {\n   self.pos.x = 1280\n   self.vel.x = -abs(self.vel.x) * BOUNCE\n  }\n  if self.pos.y < 0 {\n   self.pos.y = 0\n   self.vel.y = abs(self.vel.y) * BOUNCE\n  } else if self.pos.y > 720 {\n   self.pos.y = 720\n   self.vel.y = -abs(self.vel.y) * BOUNCE\n  }\n }\n}\nrun(var ps: list[Particle], dt: float, target: vec2, keep: float) {\n par for p in ps {\n  p.step(dt, target, keep)\n }\n}\nmain() {}\n";

#[test]
fn the_particle_step_in_a_par_for_uses_no_stack_slots() {
    let m = common::optimized(STEP);
    let body = &m.funcs[m.par_bodies[0].0 as usize].name;
    let code = dump(&m, release(), body).unwrap();
    assert!(!code.contains("explicit_slot"), "{code}");
}

#[test]
fn a_struct_store_into_a_list_checks_the_index_once() {
    let src = "struct P {\n pos: vec2\n vel: vec2\n}\ncopy(var dst: list[vec2], src: list[P]) {\n for i, p in src {\n  dst[i] = p.pos\n }\n}\nmain() {}\n";
    let m = common::optimized(src);
    let code = dump(&m, release(), "copy").unwrap();
    // One bounds check on `dst[i]` (its failure path ends in `trap user2`), not one per field.
    assert_eq!(code.matches("trap user2").count(), 1, "{code}");
}

#[test]
fn a_wrapping_multiply_never_traps() {
    // In a checked build `a * b` ends its failure path in `trap user2`; `a *% b` has none.
    let checked = Options::default();
    let m = common::optimized("pure f(a: int, b: int) -> int { a *% b }\nmain() {}\n");
    let code = dump(&m, checked, "f").unwrap();
    assert!(code.contains("imul") && !code.contains("trap user2"), "{code}");
    let m = common::optimized("pure f(a: int, b: int) -> int { a * b }\nmain() {}\n");
    let code = dump(&m, checked, "f").unwrap();
    assert_eq!(code.matches("trap user2").count(), 1, "{code}");
}
