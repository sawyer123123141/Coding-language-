//! Interface values in bodies: which expressions convert to them, which arguments are lent
//! (`iface_borrows`) rather than boxed, and calls through them (`Res::IfaceCall`).

use kestrel_check::{check, Res, Tables, Ty};
use kestrel_syntax::parse;

fn run(src: &str) -> (Option<Tables>, Vec<String>) {
    let (p, d) = parse(src);
    assert!(d.is_empty(), "parse errors: {:?}", d.iter().map(|d| &d.message).collect::<Vec<_>>());
    let (t, d) = check(&p);
    (t, d.into_iter().filter(|d| d.is_error()).map(|d| d.message).collect())
}

/// The interface, two implementors (one owning), and functions that take an interface.
const BASE: &str = "interface Damageable {
    hit(var self, n: int) -> int
    pure hp(self) -> int
}

struct Goblin: Damageable {
    health: int = 10
    hit(var self, n: int) -> int {
        self.health -= n
        self.health
    }
    pure hp(self) -> int { self.health }
}

struct Pool: Damageable {
    mem: Arena = Arena()
    hits: int = 0
    hit(var self, n: int) -> int { self.hits }
    pure hp(self) -> int { self.hits }
}

struct Holder {
    d: Damageable
    g: Goblin = Goblin()
}

show(d: Damageable) { }
heal(var d: Damageable) { }
maybe(d: Damageable?) { }
run(f: () -> int, n: int) { }
";

/// `BASE` with `body` as `main`.
fn program(body: &str) -> String {
    format!("{BASE}\nmain() {{\n{body}\n}}\n")
}

/// How many expressions are converted to an interface value or an optional of one.
fn coerced(t: &Tables) -> usize {
    t.coerce.values().filter(|ty| is_iface_target(t, **ty)).count()
}

fn is_iface_target(t: &Tables, ty: Ty) -> bool {
    match ty {
        Ty::Iface(_) => true,
        Ty::Optional(e) => matches!(t.elem(e), Ty::Iface(_)),
        _ => false,
    }
}

fn ok(src: &str) -> Tables {
    let (t, errs) = run(src);
    assert!(errs.is_empty(), "{errs:?}\n{src}");
    t.expect("tables")
}

#[test]
fn a_concrete_value_converts_wherever_an_interface_is_expected() {
    // (`Holder(d: Goblin())` also builds the default `g`, which converts nothing.)
    let sites = [
        ("let g = Goblin()\n let d: Damageable = g", 1),
        ("var d: Damageable = Goblin()\n d = Goblin()", 2),
        ("show(Goblin())", 1),
        ("var foes: list[Damageable] = []\n foes.add(Goblin())", 1),
        ("var foes: list[Damageable] = [Goblin(), Goblin()]", 2),
        ("var foes: [Damageable] = [Goblin()]", 1),
        ("var h = Holder(d: Goblin())", 1),
        ("var h = Holder(d: Goblin())\n h.d = Goblin()", 2),
        ("let o: Damageable? = Goblin()", 1),
        ("maybe(Goblin())", 1),
        ("var m: map[int, Damageable] = {}\n m[1] = Goblin()", 1),
        ("var foes: list[Damageable] = [Goblin()]\n foes.insert(0, Goblin())", 2),
    ];
    for (body, want) in sites {
        let t = ok(&program(body));
        assert_eq!(coerced(&t), want, "{body}");
    }
}

#[test]
fn a_return_converts() {
    let src = format!("{BASE}\nmake() -> Damageable {{ Goblin() }}\nearly(x: int) -> Damageable {{\n if x > 0 {{ return Goblin() }}\n Goblin()\n}}\nmain() {{ }}\n");
    let t = ok(&src);
    assert_eq!(coerced(&t), 3);
}

#[test]
fn an_interface_value_is_not_converted_again() {
    let t = ok(&program("var d: Damageable = Goblin()\n show(d)\n let o: Damageable? = d"));
    // `Goblin()` into `d`, and `d` into the optional (the existing `T` to `T?`).
    assert_eq!(coerced(&t), 2);
    assert!(t.iface_borrows.is_empty());
}

#[test]
fn only_places_passed_to_an_interface_parameter_are_lent() {
    let body = "var g = Goblin()
 var gs: list[Goblin] = [Goblin()]
 var h = Holder(d: Goblin())
 var m: map[int, Goblin] = {}
 m[1] = Goblin()
 show(g)
 show(gs[0])
 show(h.g)
 heal(var g)
 heal(var h.g)
 for x in gs { show(x) }
 show(Goblin())
 show(m.get(1, Goblin()))
 maybe(g)
 var p = Pool()
 show(p)
 heal(var p)";
    let t = ok(&program(body));
    // show(g) show(gs[0]) show(h.g) heal(var g) heal(var h.g) show(x) show(p) heal(var p)
    assert_eq!(t.iface_borrows.len(), 8);
    for e in &t.iface_borrows {
        assert!(matches!(t.coerce.get(e), Some(Ty::Iface(_))), "a lent argument is converted to the interface itself");
    }
    // Every other conversion is boxed: `show(Goblin())`, `show(m.get(...))`, `maybe(g)`, and the
    // conversions to a `Holder`'s field, which are not arguments at all.
    assert_eq!(coerced(&t), 8 + 3 + 1);
}

#[test]
fn a_call_through_an_interface_names_its_slot() {
    let t = ok(&program("var d: Damageable = Goblin()\n d.hit(1)\n let n = d.hp()"));
    let iface = t.iface_by_name["Damageable"];
    let mut slots: Vec<u32> = t
        .res
        .values()
        .filter_map(|r| match r {
            Res::IfaceCall { iface: i, slot } if *i == iface => Some(*slot),
            _ => None,
        })
        .collect();
    slots.sort_unstable();
    // Declaration order: `hit`, then `hp`.
    assert_eq!(slots, [0, 1]);
    let methods = &t.iface(iface).methods;
    assert_eq!(t.func(methods[0]).name, "Damageable.hit");
    assert_eq!(t.func(methods[1]).name, "Damageable.hp");
}

#[test]
fn a_closure_reading_an_interface_value_conflicts_with_a_var_self_call_in_the_same_call() {
    let errs = run(&program("var d: Damageable = Goblin()\n run(() { d.hp() }, d.hit(2))")).1;
    assert_eq!(errs, ["`d` is changed by this call, so the closure passed to it can't read it"]);
    // Reading it in both is fine.
    ok(&program("var d: Damageable = Goblin()\n run(() { d.hp() }, d.hp())"));
}

#[test]
fn calling_an_unknown_method_names_the_interface() {
    let errs = run(&program("var d: Damageable = Goblin()\n d.nope()")).1;
    assert_eq!(errs, ["`Damageable` has no method `nope`"]);
}

#[test]
fn nothing_converts_inside_a_container_or_to_another_interface() {
    let cases = [
        ("var gs: list[Goblin] = []\n let ds: list[Damageable] = gs", "expected list[Damageable], found list[Goblin]"),
        ("let o: Goblin? = Goblin()\n maybe(o)", "expected Damageable?, found Goblin?"),
        ("var g = Goblin()\n var o: Damageable? = Goblin()\n maybe(g)\n var gs: [Goblin] = [g]\n let ds: [Damageable] = gs", "expected [Damageable], found [Goblin]"),
    ];
    for (body, want) in cases {
        let errs = run(&program(body)).1;
        assert!(errs.iter().any(|e| e == want), "{body}: {errs:?}");
    }
}

#[test]
fn a_var_argument_converts_only_to_exactly_var_i() {
    let src = format!("{BASE}\nheal_opt(var o: Damageable?) {{ }}\nheal_list(var os: list[Damageable]) {{ }}\nmain() {{\n var g = Goblin()\n heal(var g)\n heal_opt(var g)\n heal_list(var g)\n}}\n");
    let errs = run(&src).1;
    assert_eq!(
        errs,
        ["`var` parameter `o` needs exactly Damageable?, found Goblin", "`var` parameter `os` needs exactly list[Damageable], found Goblin"]
    );
}

#[test]
fn an_owning_value_is_lent_from_a_place_and_refused_when_stored() {
    ok(&program("var p = Pool()\n show(p)\n heal(var p)"));
    let errs = run(&program("var p = Pool()\n let d: Damageable = p")).1;
    assert_eq!(errs, ["`Pool` owns an `Arena`, so it can't be stored as a `Damageable`; keep it in a `list[Pool]`"]);
    let errs = run(&program("show(Pool())")).1;
    assert_eq!(errs, ["`Pool()` owns an `Arena`; store it in a `let` first, then pass it"]);
    let errs = run(&program("var p = Pool()\n maybe(p)")).1;
    assert_eq!(errs, ["`Pool` owns an `Arena`, so it can't be stored as a `Damageable`; keep it in a `list[Pool]`"]);
}

#[test]
fn no_whole_assignment_through_a_var_interface_parameter() {
    let src = format!("{BASE}\nswap(var d: Damageable) {{ d = Goblin() }}\nmain() {{ }}\n");
    // The example is a type other than the one being assigned.
    assert_eq!(run(&src).1, ["can't replace `d`: it may be the caller's `Pool`; change it through its methods"]);
    // A plain `var` local can be assigned, and so can a field.
    ok(&program("var d: Damageable = Goblin()\n d = Goblin()\n var h = Holder(d: Goblin())\n h.d = Goblin()"));
}

#[test]
fn interface_values_cannot_be_compared_but_closures_may_capture_them() {
    let errs = run(&program("let a: Damageable = Goblin()\n let b: Damageable = Goblin()\n let same = a == b")).1;
    assert_eq!(errs, ["interface values can't be compared"]);
    let errs = run(&program("let h = Holder(d: Goblin())\n let i = Holder(d: Goblin())\n let same = h != i")).1;
    assert_eq!(errs, ["interface values can't be compared"]);
    // Rule 7b (a closure copying a struct that holds a function value) doesn't look inside interface values.
    ok(&program("let h = Holder(d: Goblin())\n let f = () { h.g.health }"));
    // The predicates behind it.
    let t = ok(&program(""));
    let holder = Ty::Struct(t.struct_by_name["Holder"]);
    assert!(t.holds_uncomparable(holder) && t.holds_iface(holder) && !t.holds_fn(holder));
}

#[test]
fn a_literal_mixing_types_needs_a_type() {
    let errs = run(&program("let foes = [Goblin(), Pool()]")).1;
    assert_eq!(errs, ["give the list a type: `var foes: list[Damageable] = [...]`"]);
}

#[test]
fn arrays_of_interface_values_cannot_be_zero_filled() {
    let errs = run(&program("let xs = array[Damageable](3)")).1;
    assert_eq!(errs, ["`array[Damageable](n)` would start with empty items; build it with a literal or `add`"]);
    let errs = run(&program("let xs = array[Holder](3)")).1;
    assert_eq!(errs, ["`array[Holder](n)` would start with empty items; build it with a literal or `add`"]);
    // An optional starts as `none`, and a list starts empty.
    ok(&program("let xs = array[Damageable?](3)\n let ys = list[Damageable]()"));
}

#[test]
fn natives_cannot_take_or_return_interface_values() {
    let src = "interface Damageable {
    pure hp(self) -> int
}
struct Goblin: Damageable {
    health: int
    pure hp(self) -> int { self.health }
}
struct Holder {
    d: Damageable
}
native a(d: Damageable)
native b(xs: list[Damageable])
native c(h: Holder)
native d() -> Damageable?
main() { }
";
    let (_, errs) = run(src);
    assert_eq!(errs, vec!["native functions can't take interface values"; 4]);
}
