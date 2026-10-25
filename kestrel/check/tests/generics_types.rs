//! Generic declarations in the checker's tables: placeholders, type lists, instances,
//! substitution and the strict (R4) answers for placeholders.

use kestrel_check::generic::{GenericOwner, Subst};
use kestrel_check::{check, implements, StructId, Tables, Ty};
use kestrel_syntax::parse;

fn tables(src: &str) -> Tables {
    let (p, d) = parse(src);
    assert!(d.is_empty(), "parse: {:?}", d.iter().map(|d| &d.message).collect::<Vec<_>>());
    let (t, d) = check(&p);
    let errors: Vec<&String> = d.iter().filter(|d| d.is_error()).map(|d| &d.message).collect();
    assert!(errors.is_empty(), "check: {errors:?}");
    t.unwrap()
}

/// The error messages for `src`, in order.
fn errors(src: &str) -> Vec<String> {
    let (p, d) = parse(src);
    assert!(d.is_empty(), "parse: {:?}", d.iter().map(|d| &d.message).collect::<Vec<_>>());
    let (_t, d) = check(&p);
    d.into_iter().filter(|d| d.is_error()).map(|d| d.message).collect()
}

fn named(t: &Tables, name: &str) -> StructId {
    let all: Vec<_> = t.structs.iter().enumerate().filter(|(_, s)| s.name == name).collect();
    assert_eq!(all.len(), 1, "one struct named {name}");
    StructId(all[0].0 as u32)
}

fn func(t: &Tables, name: &str) -> usize {
    t.fns.iter().position(|f| f.name == name).unwrap_or_else(|| panic!("no function {name}"))
}

const SRC: &str = "interface Named {\n    pure name(self) -> string\n}\nstruct Stack[T] {\n    items: list[T] = []\n    push(var self, v: T) { self.items.add(v) }\n}\nstruct Pair[A, B] {\n    a: A\n    b: B\n}\nstruct Holder[T] {\n    v: T\n    mem: Arena = Arena()\n}\nstruct Keyed[K: key, V: Named + equatable] {\n    m: map[K, int]\n}\nf(a: Stack[int], b: Stack[int], c: Stack[string], d: Pair[int, Stack[int]]) {}\ng(xs: list[int]) {\n    let hs: list[Holder[string]] = []\n}\nmain() {}\n";

#[test]
fn instances_are_interned_once() {
    let t = tables(SRC);
    named(&t, "Stack[int]");
    named(&t, "Stack[string]");
    named(&t, "Pair[int, Stack[int]]");
    let f = &t.fns[func(&t, "f")];
    assert_eq!(f.params[0].ty, f.params[1].ty);
}

#[test]
fn an_instance_has_its_fields_filled() {
    let t = tables(SRC);
    let ints = named(&t, "Stack[int]");
    assert_eq!(t.ty_name(t.strukt(ints).fields[0].ty), "list[int]");
    let pair = named(&t, "Pair[int, Stack[int]]");
    assert_eq!(t.strukt(pair).fields[1].name, "b");
    assert_eq!(t.strukt(pair).fields[1].ty, Ty::Struct(ints));
    assert!(t.strukt(ints).concrete);
    assert!(t.strukt(pair).concrete);
    assert!(t.strukt(ints).params.is_empty());
    assert_eq!(t.strukt(ints).instance.map(|i| i.def), Some(named(&t, "Stack")));
}

#[test]
fn own_placeholders_mean_the_declaration() {
    let t = tables(SRC);
    let stack = named(&t, "Stack");
    let push = &t.fns[func(&t, "Stack.push")];
    assert_eq!(push.params[0].ty, Ty::Struct(stack));
    assert_eq!(t.strukt(stack).params.len(), 1);
    assert_eq!(push.generic, Some(GenericOwner::Struct(stack)));
    assert!(push.type_params.is_empty());
    assert_eq!(push.params[1].ty, Ty::Param(t.strukt(stack).params[0]));
}

#[test]
fn an_instance_is_owning_when_its_declaration_is() {
    let t = tables(SRC);
    assert!(t.is_owning(Ty::Struct(named(&t, "Holder"))));
    assert!(t.is_owning(Ty::Struct(named(&t, "Holder[string]"))));
    assert!(!t.is_owning(Ty::Struct(named(&t, "Stack[string]"))));
    assert_eq!(t.owning_structs.len(), t.structs.len());
}

#[test]
fn substitution() {
    let mut t = tables(SRC);
    let stack = named(&t, "Stack");
    let args = t.intern_ty_list(vec![Ty::Int]);
    let s = Subst { owner: GenericOwner::Struct(stack), args };
    let field = t.strukt(stack).fields[0].ty;
    let got = t.subst(field, s);
    assert_eq!(t.ty_name(got), "list[int]");
    // The declaration's own type holds no `Param`, but still maps to the instance (P27).
    let ints = Ty::Struct(named(&t, "Stack[int]"));
    assert_eq!(t.subst(Ty::Struct(stack), s), ints);
    assert_eq!(t.subst(Ty::Int, s), Ty::Int);
    assert_eq!(t.subst_known(Ty::Struct(stack), s, "test"), ints);
    assert_eq!(t.subst_known(field, s, "test"), got);
    assert_eq!(t.ty_list(args), &[Ty::Int]);
    assert_eq!(t.intern_ty_list(vec![Ty::Int]), args);
}

#[test]
#[should_panic(expected = "this is a compiler bug")]
fn subst_known_panics_on_a_miss() {
    let mut t = tables(SRC);
    let stack = named(&t, "Stack");
    let s = Subst { owner: GenericOwner::Struct(stack), args: t.intern_ty_list(vec![Ty::Bool]) };
    t.subst_known(Ty::Struct(stack), s, "test");
}

#[test]
fn substituting_interns_new_instances() {
    let mut t = tables(SRC);
    let stack = named(&t, "Stack");
    let s = Subst { owner: GenericOwner::Struct(stack), args: t.intern_ty_list(vec![Ty::Bool]) };
    let got = t.subst(Ty::Struct(stack), s);
    let bools = named(&t, "Stack[bool]");
    assert_eq!(got, Ty::Struct(bools));
    assert_eq!(t.ty_name(t.strukt(bools).fields[0].ty), "list[bool]");
    // Interned after `compute_owning`: it still has an owning entry, and its declaration's flags.
    assert_eq!(t.owning_structs.len(), t.structs.len());
    assert!(t.strukt(bools).fields[0].default_allocates);
    assert_eq!(t.strukt(bools).methods, t.strukt(stack).methods);
}

#[test]
fn declarations_are_not_concrete() {
    let t = tables(SRC);
    let stack = named(&t, "Stack");
    assert!(!t.strukt(stack).concrete);
    assert!(!t.is_concrete(Ty::Struct(stack)));
    assert!(t.strukt(named(&t, "Stack[int]")).concrete);
    assert!(t.is_concrete(Ty::Struct(named(&t, "Stack[int]"))));
    assert!(!t.strukt(named(&t, "Pair")).concrete);
    assert!(!t.is_concrete(Ty::Param(t.strukt(stack).params[0])));
    assert!(!t.is_concrete(t.strukt(stack).fields[0].ty));
}

#[test]
fn an_instance_has_its_default_flags() {
    let t = tables(SRC);
    let stack = named(&t, "Stack");
    assert!(t.strukt(stack).fields[0].has_default && t.strukt(stack).fields[0].default_allocates);
    let strings = named(&t, "Stack[string]");
    assert!(t.strukt(strings).fields[0].has_default);
    assert!(t.strukt(strings).fields[0].default_allocates);
    // First named in a body, after collection: the flags come with its fields.
    let holder = named(&t, "Holder[string]");
    assert!(t.strukt(holder).fields[1].has_default);
    assert!(!t.strukt(holder).fields[0].has_default);
}

#[test]
fn an_instance_has_its_methods_and_interfaces() {
    let src = "interface Named {\n    pure name(self) -> string\n}\nstruct Shelf {\n    b: Box[int]\n}\nstruct Box[T]: Named {\n    v: T\n    pure name(self) -> string { \"box\" }\n}\nf(b: Box[string]) {}\nmain() {}\n";
    let t = tables(src);
    let named_iface = t.iface_by_name["Named"];
    let decl = named(&t, "Box");
    let method = t.strukt(decl).methods["name"];
    for inst in ["Box[int]", "Box[string]"] {
        let id = named(&t, inst);
        assert_eq!(t.strukt(id).methods["name"], method, "{inst}");
        assert_eq!(t.strukt(id).ifaces, vec![named_iface], "{inst}");
        assert!(implements(&t, Ty::Struct(id), named_iface), "{inst}");
    }
    assert_eq!(t.ty_name(t.strukt(named(&t, "Box[int]")).fields[0].ty), "int");
}

#[test]
fn strict_answers_for_placeholders() {
    let t = tables(SRC);
    let stack = named(&t, "Stack");
    let p = Ty::Param(t.strukt(stack).params[0]);
    assert!(t.has_heap(p));
    assert!(t.zero_fill_breaks(p));
    assert!(t.holds_iface(p));
    assert!(t.holds_uncomparable(p));
    assert!(p.is_aggregate());
    assert!(!t.holds_fn(p));
    assert!(!t.is_owning(p));
    assert!(!t.holds_arena(p));
    assert!(t.vector(p).is_none());
    assert!(!p.is_numeric());
    assert!(p.coerces_to(p));
    assert!(!p.coerces_to(Ty::Int));
    assert!(!Ty::Int.coerces_to(p));
    assert_eq!(t.ty_name(p), "T");
    assert_eq!(p.name(), "placeholder");
    // A type holding a placeholder answers through it.
    assert!(t.has_heap(t.strukt(named(&t, "Pair")).fields[0].ty));
    assert!(t.holds_uncomparable(Ty::Struct(named(&t, "Pair"))));
    let keyed = named(&t, "Keyed");
    let (k, v) = (t.strukt(keyed).params[0], t.strukt(keyed).params[1]);
    assert!(!t.holds_uncomparable(Ty::Param(v)));
    assert!(t.holds_uncomparable(Ty::Param(k)) == !t.generic.param(k).equatable);
    assert!(t.generic.param(k).equatable && t.generic.param(k).key && !t.generic.param(k).ordered);
    let named_iface = t.iface_by_name["Named"];
    assert!(implements(&t, Ty::Param(v), named_iface));
    assert!(!implements(&t, Ty::Param(k), named_iface));
    assert!(matches!(t.strukt(keyed).fields[0].ty, Ty::Map(..)));
}

#[test]
fn bad_declarations_still_resolve() {
    assert_eq!(errors("struct Two[T, T] {\n    a: T\n}\nmain() {}\n"), ["`T` is listed twice"]);
}

#[test]
fn written_types_are_counted() {
    let decls = "struct Pair[A, B] {\n    a: A\n    b: B\n}\nstruct Stack[T] {\n    v: T\n}\nstruct Enemy {\n    hp: int\n}\n";
    let cases = [
        ("a(p: Pair[int]) {}", "`Pair` takes 2 types, found 1"),
        ("a(p: Stack[int, int]) {}", "`Stack` takes 1 type, found 2"),
        ("a(p: Pair) {}", "`Pair` needs its types, like `Pair[int, int]`"),
        ("a(x: Enemy[int]) {}", "`Enemy` takes no types"),
        ("main[T]() {}", "`main` can't have placeholders"),
    ];
    for (code, want) in cases {
        let main = if code.starts_with("main") { "" } else { "main() {}\n" };
        let src = format!("{decls}{code}\n{main}");
        assert_eq!(errors(&src), [want], "{code}");
    }
}

#[test]
fn a_placeholder_is_a_map_key_only_with_key() {
    let src = "struct Grid[K] {\n    cells: map[K, int]\n}\nmain() {}\n";
    assert_eq!(errors(src), ["`K` can't be a map key without the rule `key`: write `[K: key]`"]);
}

#[test]
fn generic_functions_have_their_own_placeholders() {
    let t = tables("interface Named {\n    pure name(self) -> string\n}\npure best[T: Named + ordered](xs: list[T]) -> T? { none }\nmain() {}\n");
    let best = &t.fns[func(&t, "best")];
    assert_eq!(best.type_params.len(), 1);
    let p = best.type_params[0];
    assert_eq!(best.generic, Some(GenericOwner::Fn(kestrel_check::FnId(func(&t, "best") as u32))));
    assert_eq!(t.generic.param(p).owner, best.generic.unwrap());
    assert!(t.generic.param(p).ordered && t.generic.param(p).equatable);
    assert_eq!(t.generic.param(p).ifaces, vec![t.iface_by_name["Named"]]);
    assert_eq!(t.ty_name(best.params[0].ty), "list[T]");
    assert_eq!(t.ty_name(best.ret), "T?");
}

#[test]
fn plain_programs_make_no_instances() {
    let t = tables("struct A {\n    x: int\n}\nenum E {\n    One\n}\nmain() {}\n");
    assert_eq!(t.structs.len(), 5);
    assert_eq!(t.enums.len(), 1);
    assert!(t.generic.params.is_empty() && t.generic.ty_lists.is_empty());
    assert!(t.structs.iter().all(|s| s.concrete && s.params.is_empty() && s.instance.is_none()));
    assert!(t.fns.iter().all(|f| f.generic.is_none() && f.type_params.is_empty() && f.copy.is_none()));
}
