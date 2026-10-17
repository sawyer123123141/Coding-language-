use kestrel_check::{check, Tables};
use kestrel_syntax::parse;

fn run(src: &str) -> (Option<Tables>, Vec<String>) {
    let (p, d) = parse(src);
    assert!(d.is_empty(), "parse errors: {:?}", d.iter().map(|d| &d.message).collect::<Vec<_>>());
    let (t, d) = check(&p);
    (t, d.into_iter().map(|d| d.message).collect())
}

fn ok(src: &str) -> Tables {
    let (t, errs) = run(src);
    assert!(errs.is_empty(), "{errs:?}");
    t.unwrap()
}

#[test]
fn a_removing_loop_and_its_removes_are_recorded() {
    let t = ok("main() {\n var xs: list[int] = [1, 2]\n for x in xs { if x > 1 { remove x } }\n for y in xs { print(y) }\n}\n");
    assert_eq!(t.removing_loops.len(), 1);
    assert_eq!(t.removes.len(), 1);
    let item = *t.removes.values().next().unwrap();
    assert_eq!(t.local(item).name, "x");
}

#[test]
fn a_nested_remove_marks_only_the_loop_of_its_item() {
    let src = "main() {\n var xs: list[int] = [1, 2]\n let ys = [3]\n for x in xs {\n  for y in ys {\n   if y == x { remove x; break }\n  }\n }\n}\n";
    let t = ok(src);
    assert_eq!(t.removing_loops.len(), 1);
}

#[test]
fn the_body_may_use_the_item_its_parts_and_disjoint_fields() {
    let src = "struct Part {
 hp: int
}
enum Kind {
 Hard { hp: int }
 Soft
}
struct Brick {
 parts: list[Part]
 kind: Kind = Kind.Soft
 hp: int = 1
}
struct World {
 bricks: list[Brick]
 score: int = 0
}
main() {
 var w = World(bricks: [])
 for b in w.bricks {
  for p in b.parts { p.hp -= 1 }
  b.parts.add(Part(hp: 2))
  var bonus = 0
  match b.kind {
   Hard h { bonus = h.hp }
   Soft { }
  }
  w.score += b.hp + bonus
  if b.hp < 1 { remove b; continue }
  b.hp -= 1
 }
 print(w)
}
";
    ok(src);
}

#[test]
fn a_loop_over_an_item_of_an_outer_loop_may_remove() {
    // The inner loop changes a part of `grid`, never `grid`'s own length.
    ok("main() {\n var grid: list[list[int]] = [[1, 2], [3]]\n for row in grid {\n  for x in row { if x > 1 { remove x } }\n }\n print(grid)\n}\n");
}

#[test]
fn removing_then_leaving_the_loop_is_fine_on_every_way_out() {
    ok("f(var xs: list[int]) -> int {\n for x in xs {\n  if x == 1 { remove x; continue }\n  if x == 2 { remove x; break }\n  if x == 3 { remove x; return 3 }\n }\n 0\n}\nmain() {}\n");
}

#[test]
fn a_remove_in_an_inner_loop_followed_by_break_is_fine() {
    ok("main() {\n var xs: list[int] = [1, 2]\n for x in xs {\n  var k = 0\n  while k < 3 {\n   if k == x { remove x; break }\n   k += 1\n  }\n }\n}\n");
}

#[test]
fn the_list_is_free_to_use_after_the_loop() {
    let (_, errs) = run("main() {\n var xs: list[int] = [1, 2]\n for x in xs { remove x }\n print(xs)\n}\n");
    assert!(errs.is_empty(), "{errs:?}");
}

#[test]
fn removing_from_a_loop_over_a_string_is_refused() {
    let (_, errs) = run("main() {\n let s = \"abc\"\n for c in s { remove c }\n}\n");
    assert_eq!(errs, vec!["`remove` works in loops over a list; strings can't change, so build a new one".to_string()]);
}

#[test]
fn an_inner_loop_over_a_part_of_the_item_may_remove_it_and_break() {
    ok("struct Brick {\n hits: list[int]\n}\nmain() {\n var bs: list[Brick] = []\n for b in bs {\n  for h in b.hits {\n   if h > 3 { remove b; break }\n  }\n }\n}\n");
    // Not when the inner loop's item is used after: it read from inside the removed item.
    let (_, errs) = run("struct Brick {\n hits: list[int]\n}\nmain() {\n var bs: list[Brick] = []\n for b in bs {\n  for h in b.hits {\n   if h > 3 { remove b }\n   print(h)\n   break\n  }\n }\n}\n");
    assert_eq!(errs, vec!["`h` reads from `b`, which may have been removed earlier, so it can't be used here".to_string()]);
}

/// The message and help of each error in `src`.
fn errors_with_help(src: &str) -> Vec<(String, Option<String>)> {
    let (p, d) = parse(src);
    assert!(d.is_empty(), "parse errors: {:?}", d.iter().map(|d| &d.message).collect::<Vec<_>>());
    check(&p).1.into_iter().map(|d| (d.message, d.help)).collect()
}

#[test]
fn slips_for_remove_get_a_help_line() {
    let src = "main() {\n var xs: list[int] = [1, 2]\n for b in xs {\n  xs.remove(b)\n  xs.erase(b)\n  remove(b)\n }\n remove(xs)\n}\n";
    let help = "use `xs.remove_at(i)`, or `remove b` inside `for b in xs`".to_string();
    assert_eq!(
        errors_with_help(src),
        [
            ("`list[int]` has no method `remove`".to_string(), Some(help.clone())),
            ("`list[int]` has no method `erase`".to_string(), Some(help)),
            ("unknown function `remove`".to_string(), Some("to remove the loop's item, write `remove b` without parentheses".to_string())),
            ("unknown function `remove`".to_string(), None),
        ]
    );
}
