//! Moves follow the program's paths: a value moved on one branch is still there on the
//! other, a move followed by leaving the loop is fine, and giving a moved variable a new
//! value makes it usable again.

use kestrel_check::check;
use kestrel_syntax::parse;

/// `Bag` owns an arena, so it moves instead of copying.
const BAG: &str = "struct Bag {\n    mem: Arena = Arena()\n    n: int = 0\n}\nstruct Pair {\n    b: Bag\n}\npeek(o: Bag?) -> int {\n    1\n}\n";

fn errors(body: &str) -> Vec<String> {
    let src = format!("{BAG}{body}main() {{}}\n");
    let (p, d) = parse(&src);
    assert!(d.is_empty(), "parse errors: {:?}", d.iter().map(|d| &d.message).collect::<Vec<_>>());
    check(&p).1.into_iter().map(|d| d.message).collect()
}

const MAYBE: &str = "`t` may have been moved earlier, so it can't be used here";
const MOVED: &str = "`t` was moved earlier, so it can't be used here";
const IN_LOOP: &str = "`t` is moved inside a loop, so the next time around it would already be gone";

fn none() -> Vec<String> {
    Vec::new()
}

#[test]
fn each_branch_may_move_the_same_value() {
    assert_eq!(errors("go(c: bool) {\n var a = list[Bag]()\n var b = list[Bag]()\n let t = Bag()\n if c { a.add(t) } else { b.add(t) }\n}\n"), none());
}

#[test]
fn a_branch_that_leaves_doesnt_move_for_the_rest() {
    assert_eq!(errors("go(c: bool, var keep: list[Bag]) {\n let t = Bag()\n if c {\n  keep.add(t)\n  return\n }\n keep.add(t)\n}\n"), none());
}

#[test]
fn leaving_the_loop_right_after_a_move_is_fine() {
    let body = "go(var keep: list[Bag]) {\n let t = Bag()\n for i from 0 to 3 {\n  if i == 1 {\n   keep.add(t)\n   break\n  }\n }\n}\n";
    assert_eq!(errors(body), none());
    // After the loop, `t` was moved only if the loop reached the `break`.
    let after = "go(var keep: list[Bag]) {\n let t = Bag()\n for i from 0 to 3 {\n  if i == 1 {\n   keep.add(t)\n   break\n  }\n }\n print(t.n)\n}\n";
    assert_eq!(errors(after), vec![MAYBE.to_string()]);
}

#[test]
fn a_new_value_makes_a_moved_variable_usable() {
    assert_eq!(errors("go(var keep: list[Bag]) {\n var t = Bag()\n keep.add(t)\n t = Bag()\n print(t.n)\n}\n"), none());
    let looped = "go(var done: list[Bag]) {\n var t = Bag()\n var i = 0\n while i < 3 {\n  done.add(t)\n  t = Bag()\n  i += 1\n }\n print(t.n)\n}\n";
    assert_eq!(errors(looped), none());
    // A `continue` is a way around the loop too; the new value is there before it.
    let cont = "go(var done: list[Bag], c: bool) {\n var t = Bag()\n var i = 0\n while i < 3 {\n  i += 1\n  done.add(t)\n  t = Bag()\n  if c {\n   continue\n  }\n }\n}\n";
    assert_eq!(errors(cont), none());
}

#[test]
fn a_local_of_the_loop_body_may_move_before_an_inner_continue() {
    let body = "go(var keep: list[Bag]) {\n for i from 0 to 3 {\n  let t = Bag()\n  keep.add(t)\n  for j from 0 to 2 {\n   if j == 0 {\n    continue\n   }\n  }\n }\n}\n";
    assert_eq!(errors(body), none());
}

#[test]
fn the_else_block_of_let_else_still_has_the_optional() {
    let body = "go(var keep: list[Bag]) {\n let o: Bag? = Bag()\n let x = o else {\n  print(peek(o))\n  return\n }\n keep.add(x)\n}\n";
    assert_eq!(errors(body), none());
}

#[test]
fn the_else_block_of_let_else_leaves_so_its_moves_dont_flow_past_it() {
    let body = "go(var keep: list[Bag]) {\n let t = Bag()\n let o: Bag? = Bag()\n let x = o else {\n  keep.add(t)\n  return\n }\n keep.add(x)\n print(t.n)\n}\n";
    assert_eq!(errors(body), none());
}

#[test]
fn a_move_in_a_while_condition_stays_moved_after_the_loop() {
    // The body always leaves, so no back edge reports it; the condition's move still happened
    // on the way to the end of the loop.
    assert_eq!(
        errors("go() {\n let t = Bag()\n while Pair(b: t).b.n > 100 {\n  return\n }\n print(t.n)\n}\n"),
        vec![MOVED.to_string()]
    );
    // The right side of `or` runs only sometimes.
    assert_eq!(
        errors("go(c: bool) {\n let t = Bag()\n while c or Pair(b: t).b.n > 100 {\n  return\n }\n print(t.n)\n}\n"),
        vec![MAYBE.to_string()]
    );
}

#[test]
fn a_use_after_a_move_on_some_paths_is_an_error() {
    assert_eq!(errors("go(c: bool, var keep: list[Bag]) {\n let t = Bag()\n if c { keep.add(t) }\n print(t.n)\n}\n"), vec![MAYBE.to_string()]);
    assert_eq!(errors("go(c: bool) {\n let t = Bag()\n let ok = c and Pair(b: t).b.n > 0\n print(t.n)\n}\n"), vec![MAYBE.to_string()]);
    assert_eq!(errors("go(c: bool) {\n let t = Bag()\n let y = if c { t } else { Bag() }\n print(t.n)\n}\n"), vec![MAYBE.to_string()]);
}

#[test]
fn a_use_after_a_move_on_every_path_is_an_error() {
    assert_eq!(
        errors("go(c: bool, var a: list[Bag]) {\n let t = Bag()\n if c { a.add(t) } else { a.add(t) }\n print(t.n)\n}\n"),
        vec![MOVED.to_string()]
    );
    // Changing part of a moved variable is a use; only the whole variable gets a new value.
    assert_eq!(errors("go(var keep: list[Bag]) {\n var t = Bag()\n keep.add(t)\n t.n = 1\n}\n"), vec![MOVED.to_string()]);
    // The new value is checked first, so it can't use the moved variable.
    assert_eq!(
        errors("go(c: bool, var keep: list[Bag]) {\n var t = Bag()\n keep.add(t)\n t = if c { t } else { Bag() }\n}\n"),
        vec![MOVED.to_string()]
    );
}

const IN_PAR: &str = "`t` is declared outside this `par for`, whose iterations run at the same time, so they can't each move it";

/// The messages and help texts of each error.
fn errors_with_help(body: &str) -> Vec<(String, Option<String>)> {
    let src = format!("{BAG}{body}main() {{}}\n");
    let (p, d) = parse(&src);
    assert!(d.is_empty(), "parse errors: {:?}", d.iter().map(|d| &d.message).collect::<Vec<_>>());
    check(&p).1.into_iter().map(|d| (d.message, d.help)).collect()
}

#[test]
fn a_par_for_body_cant_move_an_outer_local() {
    let plain = "go() {\n let t = Bag()\n var outs = array[int](4)\n par for i, o in outs {\n  let m = t\n  o = m.n\n }\n}\n";
    let help = Some("declare it inside the loop, so each iteration has its own".to_string());
    assert_eq!(errors_with_help(plain), vec![(IN_PAR.to_string(), help)]);
    // A body that never finishes never goes around, but every iteration still moves `t`.
    let forever = "go() {\n let t = Bag()\n var outs = array[int](4)\n par for i, o in outs {\n  let m = t\n  while true {\n   o = m.n\n  }\n }\n}\n";
    assert_eq!(errors(forever), vec![IN_PAR.to_string()]);
    // A local of an outer `par for` body is shared by the inner one's iterations.
    let nested = "go() {\n var grid = array[[int]](2)\n par for i, row in grid {\n  let t = Bag()\n  par for j, c in row {\n   let m = t\n   c = m.n\n  }\n }\n}\n";
    assert_eq!(errors(nested), vec![IN_PAR.to_string()]);
}

#[test]
fn a_par_for_bodys_own_locals_follow_the_usual_rules() {
    let own = "go() {\n var outs = array[int](4)\n par for i, o in outs {\n  let t = Bag()\n  let m = t\n  o = m.n\n }\n}\n";
    assert_eq!(errors(own), none());
    let inner = "go() {\n var outs = array[int](4)\n par for i, o in outs {\n  for j from 0 to 2 {\n   let t = Bag()\n   let m = t\n   o = m.n\n  }\n }\n}\n";
    assert_eq!(errors(inner), none());
    // An ordinary loop inside the body still can't go around after moving the body's local.
    let around = "go() {\n var outs = array[int](4)\n par for i, o in outs {\n  let t = Bag()\n  for j from 0 to 2 {\n   let m = t\n   o = m.n\n  }\n }\n}\n";
    let help = "declare it inside the loop, give it a new value before the loop goes around, or leave the loop with `break` right after moving it";
    assert_eq!(errors_with_help(around), vec![(IN_LOOP.to_string(), Some(help.to_string()))]);
    let left = "go() {\n var outs = array[int](4)\n par for i, o in outs {\n  let t = Bag()\n  for j from 0 to 2 {\n   let m = t\n   o = m.n\n   break\n  }\n }\n}\n";
    assert_eq!(errors(left), none());
}

#[test]
fn a_loop_that_goes_around_after_a_move_is_an_error() {
    assert_eq!(errors("go(var keep: list[Bag]) {\n let t = Bag()\n for i from 0 to 3 {\n  keep.add(t)\n }\n}\n"), vec![IN_LOOP.to_string()]);
    assert_eq!(
        errors("go(var keep: list[Bag]) {\n let t = Bag()\n for i from 0 to 3 {\n  if i == 1 {\n   keep.add(t)\n   continue\n  }\n }\n}\n"),
        vec![IN_LOOP.to_string()]
    );
    // A `while` condition runs every time around.
    assert_eq!(errors("go() {\n let t = Bag()\n var i = 0\n while Pair(b: t).b.n > i {\n  i += 1\n }\n}\n"), vec![IN_LOOP.to_string()]);
    // Reported once, though both loops go around.
    assert_eq!(
        errors("go(var keep: list[Bag]) {\n let t = Bag()\n for i from 0 to 2 {\n  for j from 0 to 2 {\n   keep.add(t)\n  }\n }\n}\n"),
        vec![IN_LOOP.to_string()]
    );
    // Leaving the inner loop isn't leaving the outer one.
    assert_eq!(
        errors("go(var keep: list[Bag]) {\n let t = Bag()\n for i from 0 to 2 {\n  for j from 0 to 2 {\n   keep.add(t)\n   break\n  }\n }\n}\n"),
        vec![IN_LOOP.to_string()]
    );
}
