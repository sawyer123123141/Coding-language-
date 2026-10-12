//! Views read a place without copying it: nothing may move what a view reads, and
//! `let x = opt else` reads in place when `opt` is a place that can't change.

use kestrel_check::{check, Tables};
use kestrel_syntax::parse;

const HEAD: &str = "struct Bag {\n    mem: Arena = Arena()\n    n: int = 0\n}\nstruct Holder {\n    b: Bag?\n}\nenum Slot {\n    Empty\n    Full { b: Bag }\n}\npeek(o: Bag?) -> int {\n    1\n}\n";

fn run(body: &str) -> (Option<Tables>, Vec<String>) {
    let src = format!("{HEAD}{body}");
    let (p, d) = parse(&src);
    assert!(d.is_empty(), "parse errors: {:?}", d.iter().map(|d| &d.message).collect::<Vec<_>>());
    let (t, d) = check(&p);
    (t, d.into_iter().map(|d| d.message).collect())
}

fn errors(body: &str) -> Vec<String> {
    run(body).1
}

fn none() -> Vec<String> {
    Vec::new()
}

#[test]
fn moving_a_matched_variable_while_an_arm_reads_it_is_an_error() {
    let body = "main() {\n var keep = list[Slot]()\n let s = Slot.Full(Bag())\n match s {\n  Full f {\n   keep.add(s)\n   print(f.b.n)\n  }\n  else { }\n }\n}\n";
    assert_eq!(errors(body), vec!["can't move `s` here: this `match` arm's `f` reads from it".to_string()]);
    // An arm with no binding reads nothing.
    let free = "main() {\n var keep = list[Slot]()\n let s = Slot.Full(Bag())\n match s {\n  Full {\n   keep.add(s)\n  }\n  else { }\n }\n}\n";
    assert_eq!(errors(free), none());
}

#[test]
fn let_else_unwraps_an_owning_parameter_field_or_self_in_place() {
    assert_eq!(errors("size(o: Bag?) -> int {\n let b = o else {\n  return 0\n }\n b.n\n}\nmain() {}\n"), none());
    assert_eq!(errors("main() {\n let h = Holder(b: Bag())\n let b = h.b else {\n  return\n }\n print(b.n)\n}\n"), none());
    let method = "struct Level {\n b: Bag?\n\n size(self) -> int {\n  let b = self.b else {\n   return 0\n  }\n  b.n\n }\n}\nmain() {}\n";
    assert_eq!(errors(method), none());
}

#[test]
fn a_whole_owning_local_still_moves() {
    let body = "main() {\n var keep = list[Bag]()\n let o: Bag? = Bag()\n let b = o else {\n  return\n }\n keep.add(b)\n print(peek(o))\n}\n";
    assert_eq!(errors(body), vec!["`o` was moved earlier, so it can't be used here".to_string()]);
}

#[test]
fn a_var_unwrap_still_moves_or_copies() {
    let body = "main() {\n let h = Holder(b: Bag())\n var b = h.b else {\n  return\n }\n}\n";
    assert_eq!(
        errors(body),
        vec!["can't move this out: it holds an `Arena`, so it can't be copied, and only whole variables move".to_string()]
    );
}

#[test]
fn a_view_and_what_it_reads_cant_move() {
    let field = "main() {\n var keep = list[Bag]()\n let h = Holder(b: Bag())\n let b = h.b else {\n  return\n }\n keep.add(b)\n}\n";
    assert_eq!(errors(field), vec!["can't move `b`: it's inside `h`, and it holds an `Arena`, so it can't be copied".to_string()]);
    let param = "take(o: Bag?, var keep: list[Bag]) {\n let b = o else {\n  return\n }\n keep.add(b)\n}\nmain() {}\n";
    assert_eq!(
        errors(param),
        vec!["can't move `b`: it's inside parameter `o`, which belongs to the caller, and it holds an `Arena`, so it can't be copied".to_string()]
    );
    let root = "main() {\n var keep = list[Holder]()\n let h = Holder(b: Bag())\n let b = h.b else {\n  return\n }\n keep.add(h)\n print(b.n)\n}\n";
    assert_eq!(errors(root), vec!["can't move `h` here: `b` reads from it".to_string()]);
    // After the block that declares the view, the root may move.
    let later = "main() {\n var keep = list[Holder]()\n let h = Holder(b: Bag())\n if true {\n  let b = h.b else {\n   return\n  }\n  print(b.n)\n }\n keep.add(h)\n}\n";
    assert_eq!(errors(later), none());
}

#[test]
fn only_heap_payloads_of_places_that_cant_change_are_views() {
    let views = |body: &str| {
        let (t, errs) = run(body);
        assert!(errs.is_empty(), "{errs:?}");
        t.unwrap().let_else_views.len()
    };
    assert_eq!(views("count(name: string?) -> int {\n let n = name else {\n  return 0\n }\n 1\n}\nmain() {}\n"), 1);
    assert_eq!(views("count(name: string?) -> int {\n var n = name else {\n  return 0\n }\n 1\n}\nmain() {}\n"), 0);
    assert_eq!(views("count(o: int?) -> int {\n let n = o else {\n  return 0\n }\n n\n}\nmain() {}\n"), 0);
    // A `var` root can change while the view reads it, so it's copied.
    assert_eq!(views("main() {\n var names: string? = \"a\"\n let n = names else {\n  return\n }\n print(n)\n}\n"), 0);
}
