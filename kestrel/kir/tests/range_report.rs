//! The report behind `kestrel check --report`: allocations, other ways to fail and calls not
//! inlined, followed through calls; the order of the items; and the text.

mod range_common;

use kestrel_kir::range::render::render;
use kestrel_kir::range::report::ItemKind::{self, Allocation, Call, CanFail};
use kestrel_kir::range::Arith::{Checked, Wrapping};
use kestrel_kir::range::CheckKind;
use range_common::*;

const LIMIT: &str = "2,147,483,647";

#[test]
fn a_filter_is_an_allocation() {
    let (m, r) = reported("pub evens(xs: list[int]) -> list[int] {\n xs.filter((n) { n % 2 == 0 })\n}", Checked);
    let f = func_report(&r, "evens");
    assert_eq!(items_of(f, Allocation), [(2, "`filter` builds a new list".to_string())]);
    assert_eq!(items_of(f, CanFail), [(2, format!("`filter` fails if its list would hold more than {LIMIT} items"))]);
    assert_eq!((f.allocations(), f.fails()), (1, 1));
    assert!(r.summaries[idx(&m, "evens")].allocates.is_some());
}

#[test]
fn a_map_is_an_allocation_too() {
    let (_, r) = reported("pub doubled(xs: list[int]) -> list[int] {\n xs.map((n) { n + 1 })\n}", Checked);
    let f = func_report(&r, "doubled");
    assert_eq!(items_of(f, Allocation), [(2, "`map` builds a new list".to_string())]);
    assert_eq!(items_of(f, CanFail), [(2, format!("`map` fails if its list would hold more than {LIMIT} items"))]);
}

#[test]
fn a_filter_that_isnt_inlined_still_says_what_it_builds() {
    // Past the 8,192-node inlining budget of `many`, the calls of the generated loops stay calls.
    let mut src = String::from("pub many(xs: list[int]) -> int {\n var t = 0\n");
    for i in 0..300 {
        src.push_str(&format!(" t += xs.filter((n) {{ n % {} == 0 }}).len\n", i + 2));
    }
    src.push_str(" t\n}");
    let (m, r) = reported(&src, Checked);
    assert!(m.not_inlined.iter().any(|n| m.funcs[n.callee.0 as usize].name.starts_with("__kestrel_list_fn_filter_")), "some loop stays a call");
    let f = func_report(&r, "many");
    let allocs = items_of(f, Allocation);
    assert_eq!(allocs.len(), 300, "every filter, inlined or not");
    assert!(allocs.iter().all(|(_, t)| t == "`filter` builds a new list"), "{allocs:?}");
    // The generated loop isn't a call the program wrote: no call items.
    assert!(items_of(f, Call).is_empty(), "{:?}", items_of(f, Call));
}

#[test]
fn add_is_an_allocation_and_a_way_to_fail() {
    let src = "pub grow(var ys: list[int], n: int) {\n for i from 0 to n {\n  ys.add(i * 2)\n }\n}";
    let (_, r) = reported(src, Checked);
    let f = func_report(&r, "grow");
    assert_eq!(items_of(f, Allocation), [(3, "`add` may grow `ys`".to_string())]);
    assert_eq!(items_of(f, CanFail), [(3, format!("`add` fails if `ys` would hold more than {LIMIT} items"))]);
    // The loop is the item's group.
    assert!(f.items.iter().all(|i| i.loops.len() == 1 && line(i.loops[0]) == 2), "{:?}", f.items);
}

#[test]
fn sort_allocates() {
    let (_, r) = reported("pub ordered(var xs: list[int]) {\n xs.sort()\n}", Checked);
    let f = func_report(&r, "ordered");
    assert_eq!(items_of(f, Allocation), [(2, "`sort` makes a buffer to sort `xs` in".to_string())]);
    assert!(items_of(f, CanFail).is_empty());
    let (_, r) = reported("pub ordered(var xs: list[int]) {\n xs.sort_by((n) { 0 - n })\n}", Checked);
    let f = func_report(&r, "ordered");
    assert_eq!(items_of(f, Allocation), [(2, "`sort_by` makes a buffer to sort `xs` in".to_string())]);
}

#[test]
fn a_par_for_allocates_per_thread() {
    let src = "pub doubled(var xs: list[int]) {\n par for x in xs {\n  x = x + 1\n }\n}";
    let (_, r) = reported(src, Checked);
    let f = func_report(&r, "doubled");
    assert_eq!(items_of(f, Allocation), [(2, "a `par for` makes an arena for each thread".to_string())]);
    // The body is listed under the function, keyed by the `par for`'s line, with its own items.
    let [(at, body)] = &f.par_bodies[..] else { panic!("{:?}", f.par_bodies) };
    assert_eq!(line(*at), 2);
    let kinds: Vec<_> = body.items.iter().map(|i| (line(i.line), i.kind)).collect();
    assert_eq!(kinds, [(3, ItemKind::Check(CheckKind::Overflow))]);
    // The function's counts include the body's.
    assert_eq!(f.checks_left(), 1);
}

#[test]
fn a_chain_follows_calls() {
    let src = "spawn(var ps: list[int], v: int) -> int {\n if v < 0 { return 0 }\n ps.add(v)\n v\n}\npub run(var ps: list[int]) -> int {\n spawn(var ps, 4)\n}";
    let (m, r) = reported(src, Checked);
    let f = func_report(&r, "run");
    assert_eq!(items_of(f, Allocation), [(7, "calls `spawn` (line 7), which adds to `ps` (line 3) and may grow the list".to_string())]);
    let fail = format!("calls `spawn` (line 7), which adds to `ps` (line 3) and fails if the list would hold more than {LIMIT} items");
    assert_eq!(items_of(f, CanFail), [(7, fail)]);
    assert_eq!(items_of(f, Call), [(7, "`spawn` isn't inlined: it has an early `return`".to_string())]);
    // `spawn` itself says it directly.
    let s = func_report(&r, "spawn");
    assert_eq!(items_of(s, Allocation), [(3, "`add` may grow `ps`".to_string())]);
    let chain = r.summaries[idx(&m, "run")].allocates.as_ref().unwrap();
    assert_eq!(chain.steps.len(), 1);
}

#[test]
fn a_chain_may_have_several_calls() {
    let src = "grow(var ps: list[int]) {\n if ps.len > 9 { return }\n ps.add(1)\n}\nmid(var ps: list[int]) {\n if ps.len > 9 { return }\n grow(var ps)\n}\npub top(var ps: list[int]) {\n mid(var ps)\n}";
    let (_, r) = reported(src, Checked);
    let f = func_report(&r, "top");
    let want = "calls `mid` (line 10), which calls `grow` (line 7), which adds to `ps` (line 3) and may grow the list";
    assert_eq!(items_of(f, Allocation), [(10, want.to_string())]);
}

#[test]
fn a_cycle_allocates_if_any_function_in_it_does() {
    let src = "ping(var xs: list[int], n: int) {\n if n < 0 { return }\n pong(var xs, n - 1)\n}\npong(var xs: list[int], n: int) {\n if n < 0 { return }\n xs.add(n)\n ping(var xs, n - 1)\n}\npub go(var xs: list[int]) {\n ping(var xs, 3)\n}";
    let (m, r) = reported(src, Checked);
    for name in ["ping", "pong", "go"] {
        assert!(r.summaries[idx(&m, name)].allocates.is_some(), "`{name}` allocates");
        assert!(r.summaries[idx(&m, name)].fails.is_some(), "`{name}` can fail");
    }
    let f = func_report(&r, "ping");
    let want = "calls `pong` (line 3), which adds to `xs` (line 7) and may grow the list";
    assert_eq!(items_of(f, Allocation), [(3, want.to_string())]);
    // `pong` has a reason of its own, so its chain ends there, not back at `ping`.
    let f = func_report(&r, "pong");
    assert_eq!(items_of(f, Allocation).len(), 2, "its own add, and the call to `ping`: {:?}", f.items);
    let chain = r.summaries[idx(&m, "pong")].allocates.as_ref().unwrap();
    assert!(chain.steps.is_empty());
}

#[test]
fn a_recursive_function_that_allocates_nothing_is_clean() {
    let src = "pub fib(n: int) -> int {\n if n < 2 { return n }\n fib(n - 1) + fib(n - 2)\n}";
    let (m, r) = reported(src, Checked);
    let f = func_report(&r, "fib");
    assert!(items_of(f, Allocation).is_empty() && items_of(f, CanFail).is_empty());
    let s = &r.summaries[idx(&m, "fib")];
    assert!(s.allocates.is_none() && s.fails.is_none());
    // The two calls are listed, with why they stay.
    let why = "`fib` isn't inlined: it calls itself (it and what it calls have 1 check left)".to_string();
    assert_eq!(items_of(f, Call), [(3, why.clone()), (3, why)]);
}

#[test]
fn a_function_value_call_is_unknown() {
    let src = "pub apply(f: (int) -> int, x: int) -> int {\n if x < 0 { return 0 }\n f(x)\n}";
    let (m, r) = reported(src, Checked);
    let f = func_report(&r, "apply");
    let text = "calls the function value `f` (line 3); its body isn't known here".to_string();
    assert_eq!(items_of(f, Allocation), [(3, text.clone())]);
    assert_eq!(items_of(f, CanFail), [(3, text)]);
    // A caller that can't inline `apply` says so through it.
    let src = format!("{src}\nbump(x: int) -> int {{\n x + 1\n}}\npub user(x: int) -> int {{\n apply(bump, x)\n}}");
    let (_, r) = reported(&src, Checked);
    let f = func_report(&r, "user");
    let want = "calls `apply` (line 9), which calls the function value `f` (line 3); its body isn't known here".to_string();
    assert_eq!(items_of(f, Allocation), [(9, want.clone())]);
    assert_eq!(items_of(f, CanFail), [(9, want)]);
    assert!(r.summaries[idx(&m, "apply")].allocates.is_some());
}

fn sum_fn(name: &str, terms: usize) -> String {
    let body: Vec<String> = (1..=terms).map(|i| format!("x * {i}")).collect();
    format!("pure {name}(x: int) -> int {{ {} }}\n", body.join(" + "))
}

#[test]
fn a_big_callee_says_why_it_isnt_inlined() {
    let src = format!("{}pub user(a: int) -> int {{\n big(a)\n}}", sum_fn("big", 100));
    let (m, r) = reported(&src, Checked);
    let recorded: Vec<_> = m.not_inlined.iter().filter(|n| m.funcs[n.callee.0 as usize].name == "big").collect();
    let [n] = &recorded[..] else { panic!("{:?}", m.not_inlined) };
    assert!(n.reason.starts_with("it is ") && n.reason.ends_with(" nodes; the limit is 256"), "{}", n.reason);
    let f = func_report(&r, "user");
    let calls = items_of(f, Call);
    assert_eq!(calls.len(), 1);
    assert!(calls[0].1.starts_with(&format!("`big` isn't inlined: {}", n.reason)), "{calls:?}");
}

#[test]
fn the_budget_and_the_size_after_inlining_are_recorded() {
    // About 140 nodes a call: 80 copies pass the 8,192-node budget of `bench`.
    let calls: Vec<String> = (0..80).map(|i| format!("mid(x + {i})")).collect();
    let src = format!("{}pub bench(x: int) -> int {{\n {}\n}}", sum_fn("mid", 35), calls.join(" + "));
    let m = optimized(&src);
    let ran_out: Vec<_> = m.not_inlined.iter().filter(|n| n.reason == "the inlining budget of `bench` ran out").collect();
    assert!(!ran_out.is_empty() && ran_out.iter().all(|n| m.funcs[n.caller.0 as usize].name == "bench"), "{:?}", m.not_inlined);
    // `loop30` is small as written, but 30 inlined copies of a 40-term sum make it too big to copy.
    let uses: Vec<String> = (0..30).map(|i| format!("leaf(i + {i})")).collect();
    let src = format!(
        "{}pure loop30(n: int) -> int {{\n var t = 0\n for i from 0 to n {{\n  t += {}\n }}\n t\n}}\npub top(n: int) -> int {{\n loop30(n)\n}}",
        sum_fn("leaf", 40),
        uses.join(" + ")
    );
    let (m, r) = reported(&src, Checked);
    let recorded: Vec<_> = m.not_inlined.iter().filter(|n| m.funcs[n.callee.0 as usize].name == "loop30").collect();
    let [n] = &recorded[..] else { panic!("{:?}", m.not_inlined) };
    assert!(n.reason.starts_with("it is ") && n.reason.ends_with(" nodes after its own inlining; the limit is 4,096"), "{}", n.reason);
    let calls = items_of(func_report(&r, "top"), Call);
    assert_eq!(calls.len(), 1, "{calls:?}");
    assert!(calls[0].1.starts_with(&format!("`loop30` isn't inlined: {}", n.reason)), "{calls:?}");
}

#[test]
fn a_call_back_through_a_closure_names_the_closure() {
    // `r` -> `filter` -> the closure -> `r`: one cycle, so `r` copies `filter` one level and the call
    // to the closure inside the copy stays.
    let src = "pub r(xs: list[int], d: int) -> int {\n if d <= 0 { 0 } else { xs.filter((x) { r(xs, d - 1) > x }).len }\n}";
    let (m, r) = reported(src, Checked);
    assert!(m.not_inlined.iter().any(|n| n.reason == "it calls back into the closure at line 2"), "{:?}", m.not_inlined);
    assert!(m.not_inlined.iter().all(|n| !n.reason.contains("__kestrel")), "{:?}", m.not_inlined);
    let text = render(&r, &m, &|_| true);
    assert!(text.contains("call: the closure at line 2 isn't inlined: it calls back into the closure at line 2"), "{text}");
    assert!(!text.contains("__kestrel"), "{text}");
}

#[test]
fn a_closure_whose_budget_runs_out_is_named_as_the_report_names_it() {
    let calls: Vec<String> = (0..80).map(|i| format!("mid(x + {i})")).collect();
    let src = format!("{}pub all(xs: list[int]) -> list[int] {{\n xs.map((x) {{ {} }})\n}}", sum_fn("mid", 35), calls.join(" + "));
    let (m, _) = reported(&src, Checked);
    assert!(m.not_inlined.iter().any(|n| n.reason == "the inlining budget of the closure at line 3 ran out"), "{:?}", m.not_inlined);
    assert!(m.not_inlined.iter().all(|n| !n.reason.contains("__kestrel")), "{:?}", m.not_inlined);
}

#[test]
fn a_function_inlined_everywhere_is_never_called() {
    let src = "helper(a: int, b: int) -> int {\n a * b\n}\npub f(a: int, b: int) -> int {\n helper(a, b)\n}";
    let (m, r) = reported(src, Checked);
    let h = func_report(&r, "helper");
    assert!(h.never_called && h.items.is_empty());
    // Its check is listed where it is inlined, with the function it came from.
    let f = func_report(&r, "f");
    assert_eq!(f.items.iter().map(|i| line(i.line)).collect::<Vec<_>>(), [2]);
    let text = render(&r, &m, &|_| true);
    assert!(text.contains("  function `helper` (line 1): its own copy is never called; its checks are listed where it is inlined\n"), "{text}");
    assert!(text.contains("    line 2 (in `helper`): overflow check: `a * b` may exceed `int`"), "{text}");
    // Counted once.
    assert_eq!((r.checks, r.left, r.removed), (1, 1, 0));
}

#[test]
fn a_function_nothing_calls_is_listed_as_it_is() {
    // (Never inlined anywhere, so there is no other place its checks are listed.)
    let (_, r) = reported("dead(a: int, b: int) -> int {\n a * b\n}", Checked);
    let f = func_report(&r, "dead");
    assert!(!f.never_called);
    assert_eq!(f.checks_left(), 1);
}

#[test]
fn remove_at_and_slice_can_fail() {
    let src = "pub f(var xs: list[int], i: int, s: string, sep: string) {\n xs.remove_at(i)\n xs.insert(i, 4)\n let t = s.slice(i, 3)\n let u = s.split(sep)\n print(t, u)\n}";
    let (_, r) = reported(src, Checked);
    let f = func_report(&r, "f");
    let want = [
        (2, "`remove_at` fails if `i` is outside `xs`".to_string()),
        (3, format!("`insert` fails if the index is outside `xs` or `xs` would hold more than {LIMIT} items")),
        (4, "`s.slice(i, 3)` fails unless the range is inside `s`".to_string()),
        (5, format!("`s.split(sep)` fails if `sep` is empty or the list would hold more than {LIMIT} items")),
    ];
    assert_eq!(items_of(f, CanFail), want);
    let allocs: Vec<_> = items_of(f, Allocation).into_iter().map(|(l, _)| l).collect();
    assert_eq!(allocs, [3, 4, 5], "insert, slice and split allocate; remove_at doesn't");
}

#[test]
fn an_item_in_a_closure_names_the_closures_function() {
    let src = "helper(xs: list[int]) -> list[int] {\n xs.map((n) { n * 1000 })\n}\npub use_it(xs: list[int]) -> list[int] {\n helper(xs)\n}";
    let (m, r) = reported(src, Checked);
    // The closure's code is part of `helper` (its loop is inlined in it, so the literal's own code is
    // dead): `helper` is inlined everywhere, and its check is listed where it is inlined.
    assert!(func_report(&r, "helper").never_called);
    let u = func_report(&r, "use_it");
    assert!(u.items.iter().any(|i| matches!(i.kind, ItemKind::Check(CheckKind::Overflow)) && line(i.line) == 2), "{:?}", u.items);
    // Inlined into `use_it`, it names `helper`.
    let text = render(&r, &m, &|_| true);
    assert!(text.contains("    line 2 (in `helper`): overflow check: `n * 1000` may exceed `int`"), "{text}");
    assert!(!text.contains("__kestrel"), "{text}");
}

#[test]
fn items_come_in_line_order_with_loops_grouped() {
    let src = "pub f(var xs: list[int], n: int) {\n xs.add(n + 1)\n for i from 0 to n {\n  for j from 0 to n {\n   xs.add(i * j)\n  }\n  xs.add(i * 3)\n }\n xs.add(n + 2)\n}";
    let (m, r) = reported(src, Checked);
    let text = render(&r, &m, &|_| true);
    let want = "t.kes
  function `f` (line 1): 4 checks left (0 removed), 4 allocations, 4 other ways to fail
    line 2: overflow check: `n + 1` may exceed `int` (`n` can be any `int`)
    line 2: allocation: `add` may grow `xs`
    line 2: can fail: `add` fails if `xs` would hold more than 2,147,483,647 items
    loop at line 3
      loop at line 4
        line 5: overflow check: `i * j` may exceed `int` (`i` is 0 to 2,147,483,646; `j` is 0 to 2,147,483,646)
        line 5: allocation: `add` may grow `xs`
        line 5: can fail: `add` fails if `xs` would hold more than 2,147,483,647 items
      line 7: overflow check: `i * 3` may exceed `int` (`i` is 0 to 2,147,483,646)
      line 7: allocation: `add` may grow `xs`
      line 7: can fail: `add` fails if `xs` would hold more than 2,147,483,647 items
    line 9: overflow check: `n + 2` may exceed `int` (`n` can be any `int`)
    line 9: allocation: `add` may grow `xs`
    line 9: can fail: `add` fails if `xs` would hold more than 2,147,483,647 items
  function `main` (line 11): no checks, no allocations
total: 4 checks, 0 removed, 4 left; 4 allocations; 4 other ways to fail
";
    assert_eq!(text, want);
}

#[test]
fn a_call_statement_has_its_own_line() {
    let src = "tick(var n: int) {\n if n > 100 { return }\n n += 1\n}\nidle() {\n print(1)\n return\n}\npub main2(var n: int) {\n tick(var n)\n\n idle()\n print(2)\n idle()\n}";
    let (_, r) = reported(src, Checked);
    let f = func_report(&r, "main2");
    let calls: Vec<_> = items_of(f, Call).into_iter().map(|(l, _)| l).collect();
    assert_eq!(calls, [10, 12, 14]);
}

#[test]
fn the_release_report_has_no_overflow_items() {
    let src = "pub total(xs: list[int], k: int) -> int {\n var s = 0\n for i from 0 to xs.len {\n  s += xs[i]\n }\n s + xs[k]\n}";
    let (_, r) = reported(src, Wrapping);
    let f = func_report(&r, "total");
    let kinds: Vec<_> = f.items.iter().map(|i| (line(i.line), i.kind)).collect();
    assert_eq!(kinds, [(6, ItemKind::Check(CheckKind::Bounds))]);
}

#[test]
fn a_function_the_analysis_gave_up_on_says_so() {
    // 40 nested loops use up the budget.
    let depth = 40;
    let mut src = String::from("pub f(n: int) -> int {\n var t = 0\n");
    for d in 0..depth {
        src.push_str(&format!("var c{d} = 0\nwhile c{d} < n {{\n"));
    }
    src.push_str("t = t + 1\n");
    for d in (0..depth).rev() {
        src.push_str(&format!("c{d} += 1\n}}\n"));
    }
    src.push_str("t\n}");
    let (m, r) = reported(&src, Checked);
    let f = func_report(&r, "f");
    assert!(f.gave_up);
    assert_eq!(f.stopped, "it took too long to analyse");
    let text = render(&r, &m, &|_| true);
    assert!(text.contains("range analysis stopped early in `f` (it took too long to analyse), so every check in it stays"), "{text}");
}

#[test]
fn an_inlined_item_stays_next_to_its_call() {
    // The item of `helper` (line 2) comes between the items around the call (lines 5 and 7), not
    // before them: lines inside an inlined body say where it came from, not where it runs.
    let src = "helper(a: int, b: int) -> int {\n a * b\n}\npub f(a: int, b: int) -> int {\n let x = a + b\n let y = helper(x, b)\n y + x\n}";
    let (m, r) = reported(src, Checked);
    let f = func_report(&r, "f");
    let lines: Vec<u32> = f.items.iter().map(|i| line(i.line)).collect();
    assert_eq!(lines, [5, 2, 7]);
    let text = render(&r, &m, &|_| true);
    let at = |s: &str| text.find(s).unwrap_or_else(|| panic!("no {s:?} in\n{text}"));
    assert!(at("    line 5:") < at("    line 2 (in `helper`):") && at("    line 2 (in `helper`):") < at("    line 7:"), "{text}");
}

#[test]
fn built_in_files_are_left_out_of_the_text_and_the_total() {
    let (m, r) = reported("pub f(a: int, b: int) -> int {\n a * b\n}", Checked);
    // A user file is listed; calling the file a built-in module's leaves out its functions.
    assert!(render(&r, &m, &|_| true).contains("function `f`"));
    let text = render(&r, &m, &|file| file != 0);
    assert_eq!(text, "total: 0 checks, 0 removed, 0 left; no allocations; no other ways to fail\n");
}

#[test]
fn a_map_replaced_in_its_loop_can_fail() {
    let src = "pub f() {\n var m = {\"a\": 1}\n for k, v in m {\n  m = {\"b\": 2}\n  print(v)\n }\n}";
    let (_, r) = reported(src, Checked);
    let f = func_report(&r, "f");
    let fails = items_of(f, CanFail);
    assert!(fails.contains(&(3, "using `m[...]` fails if the map was replaced while looping over it".to_string())), "{fails:?}");
    // A map literal and a key set each allocate; the temporary map has no name.
    assert!(items_of(f, Allocation).iter().all(|(_, t)| t == "setting a key of the map may grow the map" || t == "setting a key of `m` may grow the map"), "{:?}", items_of(f, Allocation));
}

#[test]
fn arrays_arenas_strings_closures_and_interfaces_allocate() {
    let src = "interface Shape {\n pure area(self) -> int\n}\nstruct Sq: Shape {\n w: int = 1\n pure area(self) -> int { self.w * self.w }\n}\npub mk(n: int) -> (int) -> int {\n (x: int) -> int { x + n }\n}\npub boxed(w: int) -> int {\n let s: Shape = Sq(w)\n s.area()\n}\npub arr(n: int) -> int {\n let a = array[u8](n)\n a.len\n}\npub text(s: string) -> string {\n \"hi {s}\"\n}\npub scoped() -> int {\n var n = 0\n arena {\n  n = 1\n }\n n\n}";
    let (_, r) = reported(src, Checked);
    let one = |name: &str, kind| items_of(func_report(&r, name), kind);
    assert_eq!(one("mk", Allocation), [(9, "a closure that holds captured values makes an environment for them".to_string())]);
    let boxed = one("boxed", Allocation);
    assert_eq!(boxed[0], (12, "an interface value makes a box for its data".to_string()));
    assert_eq!(boxed[1], (13, "calls a method of the interface value `s` (line 13); its body isn't known here".to_string()));
    assert_eq!(one("boxed", CanFail), [(13, "calls a method of the interface value `s` (line 13); its body isn't known here".to_string())]);
    assert_eq!(one("arr", Allocation), [(16, "`array[u8](...)` makes a new array".to_string())]);
    assert_eq!(one("arr", CanFail), [(16, format!("`array[u8](...)` fails if its length is negative or more than {LIMIT}"))]);
    assert_eq!(one("text", Allocation), [(20, "string interpolation makes a new string".to_string())]);
    assert_eq!(one("scoped", Allocation), [(24, "`arena { }` makes a new arena".to_string())]);
}

#[test]
fn a_chain_takes_the_nearest_reason() {
    // `top` calls `far` first (a call away from the add) and `near` second (the add itself):
    // the chain ends at the add by the shortest way, and never goes round a cycle.
    let src = "near(var ps: list[int]) {\n if ps.len > 9 { return }\n ps.add(1)\n}\nfar(var ps: list[int]) {\n if ps.len > 9 { return }\n near(var ps)\n}\npub top(var ps: list[int]) {\n far(var ps)\n near(var ps)\n}";
    let (m, r) = reported(src, Checked);
    let chain = r.summaries[idx(&m, "top")].allocates.as_ref().unwrap();
    assert_eq!(chain.steps.len(), 1);
    assert_eq!(chain.steps[0].callee.0 as usize, idx(&m, "near"));
    let f = func_report(&r, "top");
    let allocs = items_of(f, Allocation);
    assert_eq!(allocs.len(), 2, "one for each call: {allocs:?}");
    assert_eq!(allocs[1].1, "calls `near` (line 11), which adds to `ps` (line 3) and may grow the list");
}

#[test]
fn a_closures_checks_are_part_of_its_functions_count() {
    // `any`'s loop is inlined, leaving its closure's code behind unused: the check is listed once,
    // in the function the closure is written in, and a caller counts it once.
    let src = "h_any(d: int, xs: list[int], tbl: list[int]) -> bool {\n if d > 0 { return h_any(d - 1, xs, tbl) }\n xs.any((x) { tbl[x] > 0 })\n}\npub run(xs: list[int], tbl: list[int]) -> bool {\n h_any(1, xs, tbl)\n}";
    let (_, r) = reported(src, Checked);
    let h = func_report(&r, "h_any");
    let bounds: Vec<_> = h.items.iter().filter(|i| matches!(i.kind, ItemKind::Check(CheckKind::Bounds))).collect();
    assert_eq!(bounds.len(), 1, "{:?}", h.items);
    assert_eq!(h.checks_left(), 1);
    let calls = items_of(func_report(&r, "run"), Call);
    assert_eq!(calls, [(6, "`h_any` isn't inlined: it calls itself (it and what it calls have 1 check left)".to_string())]);
}

#[test]
fn a_dead_closure_is_not_listed_beside_its_inlined_copy() {
    let src = "h(d: int) -> int {\n if d > 0 { return h(d - 1) }\n let k = 3\n let xs = [1, 2]\n let ys = xs.map((x) { k * 2 })\n ys.len\n}\npub top() -> int {\n h(1)\n}";
    let (_, r) = reported(src, Checked);
    let h = func_report(&r, "h");
    let overflows: Vec<_> = items_of(h, ItemKind::Check(CheckKind::Overflow));
    assert_eq!(overflows.len(), 1, "{:?}", h.items);
    assert_eq!(h.checks_left(), 1);
    assert_eq!(r.left, 1);
    let calls = items_of(func_report(&r, "top"), Call);
    assert_eq!(calls, [(9, "`h` isn't inlined: it calls itself (it and what it calls have 1 check left)".to_string())]);
}

#[test]
fn a_closure_that_can_fail_is_part_of_the_chain() {
    // `mk` makes a closure that slices a string: wherever the closure goes it can fail, so `mk`
    // and its callers say so.
    let src = "mk(d: int) -> (int) -> int {\n if d > 0 { return mk(d - 1) }\n (n: int) -> int { \"abc\".slice(0, n).len }\n}\npub top(d: int) -> (int) -> int {\n mk(d)\n}";
    let (m, r) = reported(src, Checked);
    assert!(r.summaries[idx(&m, "mk")].fails.is_some());
    let want = "calls `mk` (line 6), which calls the closure at line 3 (line 3), which slices a string (line 3) and fails unless the range is inside it";
    assert_eq!(items_of(func_report(&r, "top"), CanFail), [(6, want.to_string())]);
}

const ITEM: &str = "struct Item {\n id: int\n}\n";

#[test]
fn a_failing_sort_by_key_literal_is_followed() {
    let src = format!("{ITEM}h_lit(d: int) -> int {{\n if d > 0 {{ return h_lit(d - 1) }}\n var xs = list[Item]()\n xs.sort_by((it) {{ \"abc\".slice(0, it.id).len }})\n 0\n}}\npub top() -> int {{\n h_lit(1)\n}}");
    let (m, r) = reported(&src, Checked);
    let want = "calls `h_lit` (line 11), which calls the closure at line 7 (line 7), which slices a string (line 7) and fails unless the range is inside it";
    assert_eq!(items_of(func_report(&r, "top"), CanFail), [(11, want.to_string())]);
    assert!(r.summaries[idx(&m, "h_lit")].fails.is_some());
    // The comparison's own copy of the key is part of `h_lit`'s calls too.
    let own = items_of(func_report(&r, "h_lit"), CanFail);
    assert!(own.iter().any(|(l, t)| *l == 7 && t.starts_with("calls the comparison for `sort` (line 7)")), "{own:?}");
}

#[test]
fn a_failing_named_sort_by_key_is_followed_and_listed() {
    let src = format!("{ITEM}pure keyfn(it: Item) -> int {{\n \"abc\".slice(0, it.id).len\n}}\nh_named(d: int) -> int {{\n if d > 0 {{ return h_named(d - 1) }}\n var xs = list[Item]()\n xs.sort_by(keyfn)\n 0\n}}\npub top() -> int {{\n h_named(1)\n}}");
    let (m, r) = reported(&src, Checked);
    let want = "calls `h_named` (line 14), which calls the comparison for `sort` (line 10), which slices a string (line 5) and fails unless the range is inside it";
    assert_eq!(items_of(func_report(&r, "top"), CanFail), [(14, want.to_string())]);
    // The key's code is inlined into the sort's comparison, which the report doesn't list: the key
    // is listed itself, not said to be "listed where it is inlined".
    let key = func_report(&r, "keyfn");
    assert!(!key.never_called);
    assert_eq!(items_of(key, CanFail).len(), 1);
    assert!(r.summaries[idx(&m, "h_named")].fails.is_some());
}

#[test]
fn a_named_sort_by_key_that_has_a_check_counts_for_its_callers() {
    let src = format!("{ITEM}pure keyfn(it: Item) -> int {{\n var t = [1, 2, 3]\n t[it.id]\n}}\nh_named(d: int) -> int {{\n if d > 0 {{ return h_named(d - 1) }}\n var xs = list[Item]()\n xs.sort_by(keyfn)\n 0\n}}\npub top() -> int {{\n h_named(1)\n}}");
    let (_, r) = reported(&src, Checked);
    let calls = items_of(func_report(&r, "top"), Call);
    assert_eq!(calls, [(15, "`h_named` isn't inlined: it calls itself (it and what it calls have 1 check left)".to_string())]);
    assert_eq!(func_report(&r, "keyfn").checks_left(), 1);
}

#[test]
fn literals_and_calls_are_named_not_elided() {
    let src = "pub f(s: string) -> list[string] {\n s.split(\",\")\n}\npub g(var m: map[string, int]) {\n m[\"a\"] = 1\n}";
    let (_, r) = reported(src, Checked);
    let f = func_report(&r, "f");
    assert_eq!(items_of(f, CanFail), [(2, format!("`s.split(\",\")` fails if `\",\"` is empty or the list would hold more than {LIMIT} items"))]);
    let g = func_report(&r, "g");
    assert_eq!(
        items_of(g, CanFail),
        [(5, format!("setting a key of `m` fails if it adds a new key while looping over the map, or if the map would hold more than {LIMIT} keys"))]
    );
}

#[test]
fn a_par_for_with_nothing_to_list_has_no_header() {
    let (m, r) = reported("pub fill(var xs: list[int]) {\n par for x in xs {\n  x = 7\n }\n}", Checked);
    let text = render(&r, &m, &|_| true);
    assert!(text.contains("a `par for` makes an arena for each thread"), "{text}");
    assert!(!text.contains("par for at"), "{text}");
}

#[test]
fn a_closure_too_big_to_inline_is_still_listed() {
    // The `map` loop is inlined but the closure (past 256 nodes) stays a call: its code is live.
    let mut body = String::from(" var t = x\n");
    for k in 3..63 {
        body.push_str(&format!(" t = t * {k} + x\n"));
    }
    let src = format!("h(d: int, ys: list[int]) -> list[int] {{\n if d > 0 {{ return h(d - 1, ys) }}\n ys.map((x) {{\n{body} t\n }})\n}}\npub top(ys: list[int]) -> list[int] {{\n h(1, ys)\n}}");
    let (_, r) = reported(&src, Checked);
    let h = func_report(&r, "h");
    let left = h.checks_left();
    assert!(left >= 60, "the closure's checks are listed in `h`: {left}");
    let calls = items_of(func_report(&r, "top"), Call);
    assert_eq!(calls.len(), 1);
    assert!(calls[0].1.ends_with(&format!("(it and what it calls have {left} checks left)")), "{calls:?} vs {left}");
    assert_eq!(r.left, left);
}

#[test]
fn a_copy_names_the_field_it_copies() {
    let src = "struct Person {\n name: string = \"\"\n tags: list[string] = []\n}\npub dup(p: Person) -> Person {\n var q = p\n q.name = \"x\"\n q\n}";
    let (_, r) = reported(src, Checked);
    let texts: Vec<String> = items_of(func_report(&r, "dup"), Allocation).into_iter().map(|(_, t)| t).collect();
    assert!(texts.contains(&"makes a copy of the string `name`".to_string()), "{texts:?}");
    assert!(texts.contains(&"makes a copy of the list `tags`".to_string()), "{texts:?}");
}
