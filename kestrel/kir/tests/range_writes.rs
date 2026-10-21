//! What a write forgets: each test keeps a bounds check that a fact would have removed, had the
//! write been missed. Most have a near miss without the write, which proves the check.

mod range_common;

use kestrel_kir::range::Arith::Checked;
use kestrel_kir::range::CheckKind::Bounds;
use range_common::*;

/// The lines of the bounds checks left in `name`, and how many were proven.
fn bounds_of(src: &str, name: &str) -> (Vec<u32>, usize) {
    let (m, a) = analysed(src, Checked);
    let left = left_of(&m, &a, name, Bounds).iter().map(|c| line(c.line)).collect();
    (left, proven_of(&m, name, Bounds))
}

fn bounds(src: &str) -> (Vec<u32>, usize) {
    bounds_of(src, "f")
}

/// `shrink(var xs, n)` pops `n` items; recursive with an early `return`, so never inlined.
const SHRINK: &str = "shrink(var xs: list[int], n: int) {
    if n == 0 {
        return
    }
    xs.pop()
    shrink(var xs, n - 1)
}";

/// `pop_one(var xs)` pops one item and returns 1; small, so inlined.
const POP_ONE: &str = "pop_one(var xs: list[int]) -> int {
    xs.pop()
    1
}";

#[test]
fn pop_in_the_loop_forgets() {
    let src = "f(var xs: list[int]) -> int {\n var s = 0\n for i from 0 to xs.len {\n  xs.pop()\n  s += xs[i]\n }\n s\n}";
    assert_eq!(bounds(src), (vec![5], 0));
}

/// A guard on `xs`, then `{call}` (on line 11), then `xs[i]` on line 12.
fn var_call(call: &str) -> String {
    format!("{SHRINK}\nf(var xs: list[int], var ys: list[int], i: int) -> int {{\n var s = 0\n if i < xs.len and i >= 0 {{\n  {call}\n  s = xs[i]\n }}\n s\n}}")
}

#[test]
fn a_var_call_forgets() {
    assert_eq!(bounds(&var_call("shrink(var xs, 1)")), (vec![12], 0));
}

#[test]
fn an_unrelated_var_call_keeps() {
    assert_eq!(bounds(&var_call("shrink(var ys, 1)")), (vec![], 1));
}

/// A loop over a local `grid` that guards on its element's length, then `{write}` (line 6), then
/// indexes the element (line 7).
fn row_loop(write: &str) -> String {
    format!("f(n: int) -> int {{\n var grid: list[list[int]] = [[n], [n, n]]\n var s = 0\n for row in grid {{\n  if row.len > 0 {{\n   {write}\n   s += row[0]\n  }}\n }}\n s\n}}")
}

#[test]
fn a_loop_elements_list_forgets() {
    // A whole write, and a length change, of an item that may be the element.
    for write in ["grid[0] = []", "grid[0].clear()", "grid[1].pop()"] {
        assert_eq!(bounds(&row_loop(write)).0, vec![7], "{write}");
    }
}

#[test]
fn a_loop_elements_guard_proves_without_the_write() {
    assert_eq!(bounds(&row_loop("s += 1")), (vec![], 1));
}

/// `cure(var d)` empties the items of the value an interface parameter lends it; recursive, so
/// never inlined.
fn lent(between: &str) -> String {
    format!(
        "interface Healer {{
    heal(var self)
}}
struct G: Healer {{
    items: list[int]
    heal(var self) {{
        self.items.clear()
    }}
}}
cure(var d: Healer, n: int) {{
    if n == 0 {{
        d.heal()
        return
    }}
    cure(var d, n - 1)
}}
f(var g: G) -> int {{
    var s = 0
    if g.items.len > 0 {{
        {between}
        s = g.items[0]
    }}
    s
}}"
    )
}

#[test]
fn a_lent_interface_forgets() {
    assert_eq!(bounds(&lent("cure(var g, 1)")), (vec![21], 0));
}

#[test]
fn a_lent_interface_guard_proves_without_the_call() {
    assert_eq!(bounds(&lent("s = 1")), (vec![], 1));
}

#[test]
fn remove_at_forgets() {
    let src = "f(var xs: list[int]) -> int {\n var s = 0\n var i = 0\n while i < xs.len {\n  xs.remove_at(0)\n  s += xs[i]\n  i += 1\n }\n s\n}";
    assert_eq!(bounds(src), (vec![6], 0));
}

/// `n = xs.len` (at least 1), then `{between}`, then `xs[n - 1]` on line 9.
fn old_length(between: &str) -> String {
    format!("f(var xs: list[int]) -> int {{\n if xs.len == 0 {{\n  return 0\n }}\n let n = xs.len\n {between}\n var s = 0\n s += 1\n s + xs[n - 1]\n}}")
}

#[test]
fn compact_forgets() {
    let src = old_length("for b in xs {\n  if b > 2 {\n   remove b\n  }\n }");
    let (left, _) = bounds(&src);
    assert!(left.contains(&13), "the old length says nothing after the loop: {left:?}");
}

#[test]
fn the_old_length_proves_without_a_change() {
    assert_eq!(bounds(&old_length("var t = 1")), (vec![], 1));
}

#[test]
fn sort_keeps_the_length() {
    assert_eq!(bounds(&old_length("xs.sort()")), (vec![], 1));
}

#[test]
fn clear_sets_the_length() {
    assert_eq!(bounds(&old_length("xs.clear()")), (vec![9], 0));
}

#[test]
fn add_shifts_the_length() {
    let src = "f(var xs: list[int]) -> int {\n let n = xs.len\n xs.add(1)\n xs[n]\n}";
    assert_eq!(bounds(src), (vec![], 1));
    // Its near miss: one past the new end.
    let src = "f(var xs: list[int]) -> int {\n let n = xs.len\n xs.add(1)\n xs[n + 1]\n}";
    assert_eq!(bounds(src), (vec![4], 0));
}

#[test]
fn a_length_moves_only_for_the_same_list() {
    // `a` and `b` may be different rows: `a.add(0)` doesn't make `b` longer.
    let src = "f(var grid: list[list[int]]) -> int {
    var s = 0
    for a in grid {
        for b in grid {
            let n = b.len
            a.add(0)
            s += b[n]
        }
    }
    s
}";
    assert_eq!(bounds(src), (vec![7], 0));
}

#[test]
fn a_map_write_forgets_its_place() {
    let src = "struct Bag {
    items: list[int]
}
f(n: int, k: int) -> int {
    var m = map[int, Bag]()
    m[1] = Bag(items: [n])
    var s = 0
    for key, v in m {
        if v.items.len > 0 {
            m[k] = Bag(items: [])
            s += v.items[0]
        }
    }
    s
}";
    assert_eq!(bounds(src).0, vec![11]);
    // A loop that writes the map indexes it afresh at each use (lowering's `may_move`), so no fact
    // about a value's list exists to forget; without the write, the element's guard proves it.
    let read_only = src.replace("            m[k] = Bag(items: [])\n", "");
    assert_eq!(bounds(&read_only), (vec![], 1));
}

#[test]
fn assigning_an_index_variable_forgets_its_term() {
    let src = "f(grid: list[list[int]], y0: int) -> int {
    var s = 0
    var y = y0
    if y >= 0 and y + 1 < grid.len and grid[y].len > 0 {
        y += 1
        s = grid[y][0]
    }
    s
}";
    let (m, a) = analysed(src, Checked);
    let left = left_of(&m, &a, "f", Bounds);
    assert_eq!(left.len(), 1, "{left:#?}");
    assert!(line(left[0].line) == 6 && left[0].text.contains("[0]"), "{left:#?}");
}

#[test]
fn a_par_for_forgets_inner_lists() {
    let src = "struct P {
    trail: list[int]
}
f(var ps: list[P]) -> int {
    var s = 0
    if ps.len > 0 and ps[0].trail.len > 0 {
        s += ps[0].trail[0]
        par for p in ps {
            p.trail.clear()
        }
        s += ps[0].trail[0]
    }
    s
}";
    let (m, a) = analysed(src, Checked);
    let left = left_of(&m, &a, "f", Bounds);
    assert!(!left.iter().any(|c| line(c.line) == 7), "before the loop both are proven: {left:#?}");
    assert!(left.iter().any(|c| line(c.line) == 11 && c.text.contains("trail")), "{left:#?}");
}

#[test]
fn a_check_on_the_old_index_says_nothing_about_the_new() {
    let src = "f(xs: list[int], i0: int) -> int {\n var i = i0\n i = xs[i]\n xs[i]\n}";
    assert_eq!(bounds(src), (vec![3, 4], 0));
}

#[test]
fn a_condition_that_pops_after_its_guard_forgets() {
    for (kw, extra) in [("if", ""), ("while", "\n  i = 0")] {
        let src = format!(
            "{POP_ONE}\nf(var xs: list[int], i0: int) -> int {{\n var s = 0\n var i = i0\n {kw} i >= 0 and i < xs.len and pop_one(var xs) > 0 {{\n  s = xs[i]{extra}\n }}\n s\n}}"
        );
        assert_eq!(bounds(&src), (vec![9], 0), "{src}");
    }
}

#[test]
fn a_check_before_a_shrinking_call_in_the_same_statement() {
    let src = "shrink_get(var xs: list[int], n: int) -> int {
    if n == 0 {
        xs.pop()
        return 1
    }
    shrink_get(var xs, n - 1)
}
f(var xs: list[int], i: int) -> int {
    var s = 0
    if i >= 0 and i < xs.len {
        s = xs[i] + shrink_get(var xs, 1)
        s += xs[i]
    }
    s
}";
    let (left, _) = bounds(src);
    assert!(left.contains(&12), "{left:?}");
}

#[test]
fn an_inlined_get_and_a_pop_in_one_expression() {
    let get = "pure get(xs: list[int], i: int) -> int {\n xs[i]\n}";
    let src = format!(
        "{get}\n{POP_ONE}\nf(var xs: list[int], i: int) -> int {{\n var t = 0\n if i >= 0 and i < xs.len {{\n  t = get(xs, i) + pop_one(var xs)\n  t += xs[i]\n }}\n t\n}}"
    );
    let (left, _) = bounds(&src);
    assert!(left.contains(&12), "after the statement: {left:?}");
    let src = format!(
        "{get}\n{POP_ONE}\nf(var xs: list[int], i: int) -> int {{\n var t = 0\n if i >= 0 and i < xs.len {{\n  t = get(xs, i) + pop_one(var xs) + xs[i]\n }}\n t\n}}"
    );
    let (m, a) = analysed(&src, Checked);
    let left = left_of(&m, &a, "f", Bounds);
    assert!(left.iter().any(|c| line(c.line) == 11 && c.text.starts_with("`xs[i]`")), "inside it: {left:#?}");
}

/// A loop element given a whole new value, then a read through its list. Lines: the guard is on
/// 5, `{write}` on 6, the read on 7.
fn elem_assign(guard: &str, write: &str, read: &str) -> String {
    format!(
        "f(n: int) -> int {{\n var grid: list[list[int]] = [[n, n, n], [n, n, n]]\n var s = 0\n for k, row in grid {{\n  if {guard} {{\n   {write}\n   s += {read}\n  }}\n }}\n s\n}}"
    )
}

#[test]
fn assigning_a_loop_element_forgets_the_lengths_its_list_reaches() {
    // `row = []` writes `grid[k]`, and may write any other element of `grid`.
    for write in ["row = []", "row = [n]"] {
        assert_eq!(bounds(&elem_assign("grid[k].len > 2", write, "grid[k][2]")).0, vec![7], "{write}");
    }
    let src = "f(n: int) -> int {
    var grid: list[list[int]] = [[n, n, n], [n, n, n]]
    var s = 0
    for a in grid {
        for b in grid {
            if b.len > 2 {
                a = []
                s += b[2]
            }
        }
    }
    s
}";
    assert_eq!(bounds(src).0, vec![8]);
    // A field inside the element.
    let src = "struct Part {
    items: list[int]
}
f(n: int) -> int {
    var parts = [Part(items: [n, n, n]), Part(items: [n])]
    var s = 0
    for k, p in parts {
        if parts[k].items.len > 2 {
            p = Part(items: [])
            s += parts[k].items[2]
        }
    }
    s
}";
    assert_eq!(bounds(src).0, vec![10]);
}

#[test]
fn assigning_a_loop_element_still_knows_its_new_length() {
    // The near misses: without the write the guard proves the read, and the element's own new
    // length is known.
    assert_eq!(bounds(&elem_assign("grid[k].len > 2", "s += 1", "grid[k][2]")).0, vec![]);
    assert_eq!(bounds(&elem_assign("n > 0", "row = [n, n]", "row[1]")).0, vec![]);
}

/// `cure_now` is inlined: its interface parameter becomes a local holding `g` lent, and its other
/// argument checks `g.items[1]` (line 14) after that local is made, before `d.heal()` empties `g`.
fn lent_inlined(read: &str) -> String {
    format!(
        "interface Healer {{
    heal(var self)
}}
struct G: Healer {{
    items: list[int]
    heal(var self) {{
        self.items.clear()
    }}
}}
cure_now(var d: Healer, n: int) {{
    d.heal()
}}
f(var g: G) -> int {{
    cure_now(var g, {read})
    g.items[1]
}}"
    )
}

#[test]
fn an_inlined_lent_interface_forgets_at_its_virtual_call() {
    assert_eq!(bounds(&lent_inlined("g.items[1]")).0, vec![14, 15]);
    // Its near miss: with no check in the argument, nothing is known about `g.items` anyway.
    assert_eq!(bounds(&lent_inlined("0")).0, vec![15]);
}
