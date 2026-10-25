//! The collection pass (R2, R6): after every body is checked, one `FnInfo` per copy (a generic
//! function or method and a set of types), every type a copy's lowering reads interned (G13),
//! and the endless-copy checks: the static function check and the caps.

use kestrel_check::{check_keeping_tables, FnId, Subst, Tables, Ty};
use kestrel_syntax::ast::{Expr, ExprKind, Item, Program};
use kestrel_syntax::parse;
use kestrel_syntax::visit::{each_expr, each_expr_in};

fn run(src: &str) -> (Program, Tables, Vec<String>) {
    let (p, d) = parse(src);
    assert!(d.is_empty(), "parse errors: {:?}", d.iter().map(|d| &d.message).collect::<Vec<_>>());
    let (t, d) = check_keeping_tables(&p, true);
    let errs = d.into_iter().filter(|d| d.is_error()).map(|d| d.message).collect();
    (p, t, errs)
}

fn clean(src: &str) -> (Program, Tables) {
    let (p, t, errs) = run(src);
    assert!(errs.is_empty(), "{errs:?}");
    (p, t)
}

/// The names of the copies, in the order they were made.
fn copies(t: &Tables) -> Vec<String> {
    t.fns.iter().filter(|f| f.copy.is_some()).map(|f| f.name.clone()).collect()
}

fn fn_named(t: &Tables, name: &str) -> FnId {
    let found: Vec<usize> = t.fns.iter().enumerate().filter(|(_, f)| f.name == name).map(|(i, _)| i).collect();
    assert_eq!(found.len(), 1, "functions named `{name}`: {found:?}");
    FnId(found[0] as u32)
}

#[test]
fn one_copy_per_type() {
    let src = "struct Enemy {\n    hp: int\n}\npure best[T](xs: list[T]) -> T {\n    xs[0]\n}\nmain() {\n    let a: list[int] = [1, 2]\n    let b: list[int] = [3]\n    let foes: list[Enemy] = [Enemy(hp: 3)]\n    print(best(a))\n    print(best(b))\n    print(best(foes).hp)\n    let f = best[int]\n    print(f(a))\n}\n";
    let (_, t) = clean(src);
    assert_eq!(copies(&t), ["best[int]", "best[Enemy]"]);
    assert_eq!(t.generic.fn_copies.len(), 2);
    // The copies are entered under their generic and types.
    let best = fn_named(&t, "best");
    for (&(generic, args), &copy) in &t.generic.fn_copies {
        assert_eq!(generic, best);
        let info = t.func(copy).copy.expect("a copy");
        assert_eq!((info.generic, info.args), (generic, args));
    }
}

#[test]
fn an_unused_generic_has_no_copies() {
    let src = "struct Stack[T] {\n    items: list[T] = []\n    size(self) -> int {\n        self.items.len\n    }\n}\npure best[T](xs: list[T]) -> T {\n    xs[0]\n}\nmain() {}\n";
    let (_, t) = clean(src);
    assert!(copies(&t).is_empty(), "{:?}", copies(&t));
    assert!(t.generic.fn_copies.is_empty());
}

const OUTER_INNER: &str = "pure outer[T](x: T) -> int {\n    inner([x])\n}\npure inner[T](xs: list[T]) -> int {\n    xs.len\n}\nmain() {\n    print(outer(1))\n}\n";

#[test]
fn copies_make_copies() {
    let (_, t) = clean(OUTER_INNER);
    assert_eq!(copies(&t), ["outer[int]", "inner[int]"]);
}

#[test]
fn made_at_and_made_by() {
    let (p, t) = clean(OUTER_INNER);
    let (outer, inner) = (fn_named(&t, "outer[int]"), fn_named(&t, "inner[int]"));
    let main = p.items.iter().find_map(|i| match i {
        Item::Fn(f) if f.name == "main" => Some(f),
        _ => None,
    });
    let mut call = None;
    each_expr(&main.expect("main").body, &mut |e: &Expr| {
        if matches!(&e.kind, ExprKind::Call { name, .. } if name == "outer") {
            call = Some(e.span);
        }
    });
    let call = call.expect("the call of `outer`");
    let outer_copy = t.func(outer).copy.expect("a copy");
    let inner_copy = t.func(inner).copy.expect("a copy");
    assert_eq!(outer_copy.made_by, None);
    assert_eq!(outer_copy.made_at, call);
    assert_eq!(inner_copy.made_by, Some(outer));
    assert_eq!(inner_copy.made_at, call);
}

#[test]
fn method_copies() {
    let src = "struct Stack[T] {\n    items: list[T] = []\n    push(var self, v: T) {\n        self.items.add(v)\n    }\n}\nmain() {\n    var s = Stack[int]()\n    s.push(3)\n    print(s.items)\n}\n";
    let (_, t) = clean(src);
    assert_eq!(copies(&t), ["Stack[int].push"]);
    let copy = t.func(fn_named(&t, "Stack[int].push"));
    let stack_int = t.structs.iter().position(|s| s.name == "Stack[int]").expect("the instance");
    assert_eq!(copy.params[0].ty, Ty::Struct(kestrel_check::StructId(stack_int as u32)));
    assert_eq!(copy.params[1].ty, Ty::Int);
    assert!(copy.type_params.is_empty());
    // A copy keeps its generic's owner and locals (G24).
    let generic = t.func(copy.copy.expect("a copy").generic);
    assert_eq!(copy.generic, generic.generic);
    assert_eq!(copy.locals, generic.locals);
}

const DAMAGE: &str = "interface Damageable {\n    pure hp(self) -> int\n}\nstruct Goblin: Damageable {\n    life: int\n    pure hp(self) -> int {\n        self.life\n    }\n}\nstruct Bag[T]: Damageable {\n    item: T\n    pure hp(self) -> int {\n        1\n    }\n}\npure total_hp[T: Damageable](xs: list[T]) -> int {\n    var n = 0\n    for x in xs {\n        n = n + x.hp()\n    }\n    n\n}\n";

#[test]
fn rule_calls_reach_concrete_methods() {
    let goblins = format!("{DAMAGE}main() {{\n    let gs: list[Goblin] = [Goblin(life: 3)]\n    print(total_hp(gs))\n}}\n");
    let (_, t) = clean(&goblins);
    assert_eq!(copies(&t), ["total_hp[Goblin]"]);
    let bags = format!("{DAMAGE}main() {{\n    let bs: list[Bag[int]] = [Bag[int](item: 1)]\n    print(total_hp(bs))\n}}\n");
    let (_, t) = clean(&bags);
    assert_eq!(copies(&t), ["total_hp[Bag[int]]", "Bag[int].hp"]);
    let ifaces = format!("{DAMAGE}main() {{\n    let ds: list[Damageable] = [Goblin(life: 3)]\n    print(total_hp(ds))\n}}\n");
    let (_, t) = clean(&ifaces);
    assert_eq!(copies(&t), ["total_hp[Damageable]"]);
}

#[test]
fn an_interface_conversion_copies_the_table() {
    let src = "interface Named {\n    name(self) -> string\n}\nstruct Stack[T]: Named {\n    items: list[T] = []\n    name(self) -> string {\n        \"stack\"\n    }\n}\nshow(n: Named) {\n    print(n.name())\n}\nmain() {\n    let s = Stack[int]()\n    show(s)\n}\n";
    let (_, t) = clean(src);
    assert_eq!(copies(&t), ["Stack[int].name"]);
}

#[test]
fn defaults_and_constants_are_roots() {
    let src = "pure count[T](xs: list[T]) -> int {\n    xs.len\n}\npure best[T](xs: list[T]) -> T {\n    xs[0]\n}\nstruct Counter {\n    measure: pure (list[int]) -> int = (xs) { count(xs) }\n}\nlet RULE = best[int]\nmain() {\n    let c = Counter()\n    print(c.measure([1]))\n}\n";
    let (_, t) = clean(src);
    let mut made = copies(&t);
    made.sort();
    assert_eq!(made, ["best[int]", "count[int]"]);
}

#[test]
fn every_type_a_copy_reads_is_interned() {
    let src = "interface Named {\n    name(self) -> string\n}\nstruct Stack[T]: Named {\n    items: list[T] = []\n    push(var self, v: T) {\n        self.items.add(v)\n    }\n    name(self) -> string {\n        \"stack\"\n    }\n    pure top(self) -> T? {\n        if self.items.len == 0 {\n            return none\n        }\n        self.items[self.items.len - 1]\n    }\n}\npure apply[T](xs: list[T], f: pure (T) -> T) -> list[T] {\n    var out: list[T] = []\n    let twice = (y: T) { f(f(y)) }\n    for x in xs {\n        out.add(twice(x))\n    }\n    out\n}\nshow(n: Named) {\n    print(n.name())\n}\nmain() {\n    var s = Stack[float]()\n    s.push(2.5)\n    show(s)\n    let ys = apply([1, 2], (x) { x + 1 })\n    print(ys)\n    print(s.top())\n}\n";
    let (p, t) = clean(src);
    let made = copies(&t);
    for name in ["Stack[float].push", "Stack[float].name", "Stack[float].top", "apply[int]"] {
        assert!(made.iter().any(|m| m == name), "no copy `{name}` in {made:?}");
    }
    for info in t.fns.iter().filter(|f| f.copy.is_some()) {
        let copy = info.copy.expect("a copy");
        let s = Subst { owner: info.generic.expect("a copy keeps its owner"), args: copy.args };
        for &l in &info.locals {
            t.subst_known(t.local(l).ty, s, &info.name);
        }
        let mut exprs = Vec::new();
        each_expr(&info.decl(&p).body, &mut |e: &Expr| exprs.push(e.id));
        for e in exprs {
            assert!(t.is_concrete(t.subst_known(t.ty(e), s, &info.name)));
            if let Some(&c) = t.coerce.get(&e) {
                t.subst_known(c, s, &info.name);
            }
            if let Some(&l) = t.generic.type_args.get(&e) {
                for &a in t.ty_list(l) {
                    t.subst_known(a, s, &info.name);
                }
            }
            if let Some(c) = t.closures.get(&e) {
                t.subst_known(Ty::Fn(c.sig), s, &info.name);
                for &l in &c.locals {
                    t.subst_known(t.local(l).ty, s, &info.name);
                }
                for cap in &c.captures {
                    t.subst_known(cap.ty, s, &info.name);
                }
            }
        }
    }
    // Every instance interned late has its owning entry (a missing one reads as not owning).
    assert_eq!(t.owning_structs.len(), t.structs.len());
    assert_eq!(t.owning_enums.len(), t.enums.len());
    // A default walked under an instance's types reads its types too.
    let stack = p.items.iter().find_map(|i| match i {
        Item::Struct(s) if s.name == "Stack" => Some(s),
        _ => None,
    });
    let float = t.structs.iter().position(|s| s.name == "Stack[float]").expect("the instance");
    let inst = t.structs[float].instance.expect("an instance");
    let s = Subst { owner: kestrel_check::GenericOwner::Struct(inst.def), args: inst.args };
    let default = stack.expect("Stack").fields[0].default.as_ref().expect("a default");
    each_expr_in(default, &mut |e: &Expr| {
        t.subst_known(t.ty(e.id), s, "Stack[float]");
    });
}

#[test]
fn the_cap_names_the_chain() {
    let src = "interface Grow {\n    pure grow(self) -> int\n}\nstruct Box[T]: Grow {\n    v: T\n    pure grow(self) -> int { walk(Box[Box[T]](v: self)) }\n}\npure walk[T: Grow](x: T) -> int { x.grow() }\nmain() {\n    print(walk(Box(v: 1)))\n}\n";
    let (_, _, errs) = run(src);
    assert_eq!(errs, ["making copies of `walk` doesn't end: `walk[Box[int]]` needs `walk[Box[Box[int]]]`, and so on (stopped after 32 levels)"]);
}

#[test]
fn the_cap_stops_growing_defaults() {
    let src = "struct S[T] {\n    f: pure () -> int = () {\n        let inner = S[list[T]]()\n        1\n    }\n}\nmain() {\n    let s = S[int]()\n    print(s.f())\n}\n";
    let (_, _, errs) = run(src);
    assert_eq!(errs, ["making copies of `S` doesn't end: `S[int]` needs `S[list[int]]`, and so on (stopped after 32 levels)"]);
}

#[test]
fn the_copy_count_cap() {
    // Each copy of `walk` leads to two more, through rule calls: 10,000 copies come long before
    // 32 levels, and the walk stops there instead of running on.
    let src = "interface Grow {\n    pure grow(self) -> int\n}\nstruct L[T]: Grow {\n    v: T\n    pure grow(self) -> int { walk(L[L[T]](v: self)) + walk(R[L[T]](v: self)) }\n}\nstruct R[T]: Grow {\n    v: T\n    pure grow(self) -> int { walk(L[R[T]](v: self)) + walk(R[R[T]](v: self)) }\n}\npure walk[T: Grow](x: T) -> int { x.grow() }\nmain() {\n    print(walk(L(v: 1)))\n}\n";
    let (_, t, errs) = run(src);
    assert_eq!(errs.len(), 1, "{errs:?}");
    let start = "making copies of `walk` doesn't end: `walk[L[int]]` needs `walk[";
    assert!(errs[0].starts_with(start) && errs[0].ends_with("]]]`, and so on (stopped after 10,000 copies)"), "{}", errs[0]);
    assert_eq!(copies(&t).iter().filter(|c| c.starts_with("walk[")).count(), 10_000);
}

#[test]
fn enum_method_copies() {
    let src = "enum Maybe[T] {\n    Nothing\n    Just { value: T }\n    pure tag(self) -> int {\n        1\n    }\n}\nmain() {\n    let m = Maybe[int].Just(value: 1)\n    let n: Maybe[string] = Maybe.Nothing\n    print(m.tag() + n.tag())\n}\n";
    let (_, t) = clean(src);
    assert_eq!(copies(&t), ["Maybe[int].tag", "Maybe[string].tag"]);
}

#[test]
fn the_static_check_refuses_a_growing_call() {
    let src = "pure deeper[T](x: T, n: int) -> int {\n    if n == 0 { return 0 }\n    let xs: list[T] = [x]\n    deeper(xs, n - 1)\n}\nmain() {\n    print(deeper(1, 3))\n}\n";
    let (_, t, errs) = run(src);
    assert_eq!(errs, ["`deeper` would need endless copies: `deeper[T]` calls `deeper[list[T]]`, which calls `deeper[list[list[T]]]`, and so on"]);
    // The walk doesn't run after it (P9).
    assert!(copies(&t).is_empty());
}

#[test]
fn constructions_are_not_edges() {
    // `Box.wrap` builds a `Box[Box[T]]` but calls nothing generic with it: no endless copies (P24).
    let src = "struct Box[T] {\n    v: T\n    pure wrap(self) -> int {\n        let b = Box[Box[T]](v: self)\n        1\n    }\n}\nmain() {\n    let b = Box(v: 1)\n    print(b.wrap())\n}\n";
    let (_, t) = clean(src);
    assert_eq!(copies(&t), ["Box[int].wrap"]);
}

#[test]
fn no_generics_no_walk() {
    let src = "struct Enemy {\n    hp: int\n}\npure best(xs: list[Enemy]) -> Enemy {\n    xs[0]\n}\nmain() {\n    let foes: list[Enemy] = [Enemy(hp: 3)]\n    print(best(foes).hp)\n}\n";
    let (_, t) = clean(src);
    assert!(copies(&t).is_empty());
    assert!(t.generic.params.is_empty() && t.generic.ty_lists.is_empty() && t.generic.fn_copies.is_empty());
    assert!(t.structs.iter().all(|s| s.instance.is_none()));
}
