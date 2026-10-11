//! `GpuHandle`: an owning id of a GPU object, only for built-in modules. A struct that
//! holds one moves like a struct that holds an `Arena`, but may be overwritten.

use kestrel_check::{check, Tables, Ty};
use kestrel_syntax::parse;

/// Checks `src` as if the whole file were a built-in module (`builtin`) or a user's file.
fn run(src: &str, builtin: bool) -> (Option<Tables>, Vec<(u32, String)>) {
    let (mut p, d) = parse(src);
    assert!(d.is_empty(), "parse errors: {:?}", d.iter().map(|d| &d.message).collect::<Vec<_>>());
    if builtin {
        p.builtin_ranges = std::iter::once(0..src.len() as u32).collect();
    }
    let (t, d) = check(&p);
    (t, d.into_iter().map(|d| (d.span.start, d.message)).collect())
}

fn errors(src: &str, builtin: bool) -> Vec<String> {
    run(src, builtin).1.into_iter().map(|(_, m)| m).collect()
}

const TEX: &str = "struct Tex {
    id: GpuHandle
    w: int
}
struct Holder {
    t: Tex
    n: int
}
native make(w: int) -> Tex
native take(t: Tex) -> int
";

/// `body` after the declarations, checked as a built-in module.
fn builtin_errors(body: &str) -> Vec<String> {
    errors(&format!("{TEX}{body}"), true)
}

#[test]
fn gpu_structs_check_as_natives_params_and_results() {
    let src = format!("{TEX}main() {{\n let t = make(1)\n print(take(t), t.w)\n}}\n");
    let (t, errs) = run(&src, true);
    assert!(errs.is_empty(), "{errs:?}");
    let t = t.unwrap();
    let tex = t.locals.iter().find(|l| l.name == "t").unwrap().ty;
    assert!(t.is_owning(tex), "a struct holding a GpuHandle is owning");
    assert!(!t.holds_arena(tex));
    assert!(!t.has_heap(tex));
    assert!(t.is_owning(Ty::Gpu));
    assert!(!t.holds_arena(Ty::Gpu));
    assert_eq!(t.owns_what(tex), "a GPU object");
    assert_eq!(t.owns_what(Ty::Arena), "an `Arena`");
}

#[test]
fn a_gpu_object_moves_and_cannot_be_used_after() {
    assert_eq!(
        builtin_errors("main() {\n let a = make(1)\n let b = a\n print(a)\n}\n"),
        vec!["`a` was moved earlier, so it can't be used here".to_string()]
    );
}

#[test]
fn a_place_holding_a_gpu_object_may_be_overwritten() {
    assert!(builtin_errors("main() {\n var t = make(1)\n t = make(2)\n}\n").is_empty());
    assert!(builtin_errors("main() {\n var h = Holder(t: make(1), n: 1)\n h.t = make(3)\n}\n").is_empty());
    assert!(builtin_errors("main() {\n var ts = list[Tex]()\n ts.add(make(1))\n ts[0] = make(2)\n}\n").is_empty());
}

#[test]
fn a_gpu_object_cannot_be_copied_out_of_a_place() {
    let moved_out = "can't move this out: it holds a GPU object, so it can't be copied, and only whole variables move";
    assert_eq!(
        builtin_errors("main() {\n var ts = list[Tex]()\n ts.add(make(1))\n let t = ts[0]\n}\n"),
        vec![moved_out.to_string()]
    );
    assert_eq!(
        builtin_errors("main() {\n let h = Holder(t: make(1), n: 1)\n let t = h.t\n}\n"),
        vec![moved_out.to_string()]
    );
    assert_eq!(
        builtin_errors("f(t: Tex) -> Tex {\n t\n}\nmain() {}\n"),
        vec!["can't move parameter `t`: it belongs to the caller, and it holds a GPU object, so it can't be copied".to_string()]
    );
    assert_eq!(
        builtin_errors("main() {\n var ts = list[Tex]()\n ts.add(make(1))\n for t in ts {\n  var all = list[Tex]()\n  all.add(t)\n }\n}\n"),
        vec!["can't move `t` out of its collection: it holds a GPU object, so it can't be copied".to_string()]
    );
}

#[test]
fn maps_cannot_hold_gpu_objects() {
    assert_eq!(
        builtin_errors("main() {\n var m = map[int, Tex]()\n}\n"),
        vec![
            "maps can't hold values that own a GPU object yet (`Tex` has one); keep them in a list and map keys to indexes"
                .to_string()
        ]
    );
}

#[test]
fn arena_messages_keep_their_text_when_a_struct_holds_both() {
    let both = "struct Both {\n g: Tex\n mem: Arena = Arena()\n}\n";
    assert_eq!(
        builtin_errors(&format!("{both}main() {{\n let b = Both(g: make(1))\n let g = b.g\n}}\n")),
        vec!["can't move this out: it holds a GPU object, so it can't be copied, and only whole variables move".to_string()]
    );
    assert_eq!(
        builtin_errors(&format!("{both}main() {{\n let b = Both(g: make(1))\n let m = b\n let p = b\n}}\n")),
        vec!["`b` was moved earlier, so it can't be used here".to_string()]
    );
    assert_eq!(
        builtin_errors(&format!("{both}f(b: Both) -> Both {{\n b\n}}\nmain() {{}}\n")),
        vec!["can't move parameter `b`: it belongs to the caller, and it holds an `Arena`, so it can't be copied".to_string()]
    );
    assert_eq!(
        builtin_errors(&format!("{both}main() {{\n var m = map[int, Both]()\n}}\n")),
        vec![
            "maps can't hold values that own an arena yet (`Both` has one); keep them in a list and map keys to indexes"
                .to_string()
        ]
    );
}

#[test]
fn a_gpu_handle_cannot_be_used_in_a_user_file() {
    let src = "struct Mine {\n h: GpuHandle\n}\nmain() {}\n";
    let (t, errs) = run(src, false);
    assert!(t.is_none());
    let at = src.find("GpuHandle").unwrap() as u32;
    assert_eq!(errs, vec![(at, "`GpuHandle` is only for Kestrel's built-in modules".to_string())]);
}

#[test]
fn a_gpu_handle_in_a_parameter_or_local_type_is_also_refused_in_a_user_file() {
    let errs = errors("f(h: GpuHandle) {}\nmain() {}\n", false);
    assert_eq!(errs, vec!["`GpuHandle` is only for Kestrel's built-in modules".to_string()]);
}

#[test]
fn a_gpu_handle_in_a_module_after_the_built_in_range_is_refused() {
    // Only text inside a built-in range counts; the program is `<builtin source><user source>`.
    let builtin = "struct Tex {\n id: GpuHandle\n}\n";
    let src = format!("{builtin}struct Mine {{\n h: GpuHandle\n}}\nmain() {{}}\n");
    let (mut p, d) = parse(&src);
    assert!(d.is_empty());
    p.builtin_ranges = std::iter::once(0..builtin.len() as u32).collect();
    let (_, d) = check(&p);
    let at = src.rfind("GpuHandle").unwrap() as u32;
    let got: Vec<_> = d.iter().map(|d| (d.span.start, d.message.as_str())).collect();
    assert_eq!(got, vec![(at, "`GpuHandle` is only for Kestrel's built-in modules")]);
}

#[test]
fn gpu_handle_is_a_reserved_type_name() {
    let src = "struct GpuHandle {\n x: int\n}\nmain() {}\n";
    for builtin in [false, true] {
        assert_eq!(errors(src, builtin), vec!["`GpuHandle` is a built-in name; pick another".to_string()], "builtin: {builtin}");
    }
}

#[test]
fn in_builtin_tells_whether_a_span_starts_in_a_built_in_module() {
    let src = format!("{TEX}main() {{}}\n");
    let (t, errs) = run(&src, true);
    assert!(errs.is_empty(), "{errs:?}");
    let t = t.unwrap();
    let span = |start: u32| kestrel_syntax::Span::new(start as usize, start as usize + 1);
    assert!(t.in_builtin(span(0)));
    assert!(t.in_builtin(span(src.len() as u32 - 1)));
    assert!(!t.in_builtin(span(src.len() as u32)));
    let (t, _) = run("main() {}\n", false);
    assert!(!t.unwrap().in_builtin(span(0)));
}

#[test]
fn a_gpu_handle_cannot_be_built_compared_or_copied() {
    let moved_out = "can't move this out: it holds a GPU object, so it can't be copied, and only whole variables move";
    let cases: &[(&str, &str)] = &[
        ("let a = make(1)\n let b = make(2)\n print(a.id == b.id)", "`==` needs numbers, found GpuHandle and GpuHandle"),
        ("let a = make(1)\n print(a.id + 1)", "`+` needs numbers, found GpuHandle and int"),
        ("let a = GpuHandle()", "unknown function `GpuHandle`"),
        ("let a = make(1)\n let x = a.id", moved_out),
        ("let a = make(1)\n var t = make(2)\n t.id = a.id", moved_out),
    ];
    for (body, expected) in cases {
        assert_eq!(builtin_errors(&format!("main() {{\n {body}\n}}\n")), vec![expected.to_string()], "in: {body}");
    }
    // Reading it to print is fine, and so is passing the struct to a function that only reads it.
    let reads = "show(t: Tex) {\n print(t.id)\n}\nmain() {\n let a = make(1)\n show(a)\n show(a)\n print(a.id)\n}\n";
    assert!(builtin_errors(reads).is_empty());
}

#[test]
fn an_owning_part_of_a_temporary_cannot_be_stored() {
    // A part of a call's result or a construction could outlive the rest, which is dropped at
    // the end of the block (and a part may hold memory from a sibling arena), so it is refused.
    let moved_out = "can't move this out: it holds a GPU object, so it can't be copied, and only whole variables move";
    let decls = "mk() -> Holder {\n Holder(t: make(1), n: 1)\n}\nmany() -> list[Tex] {\n var xs = list[Tex]()\n xs.add(make(1))\n xs\n}\n";
    let stores = [
        "let t = mk().t",
        "let t = many()[0]",
        "let t = Holder(t: make(1), n: 1).t",
        "var ts = list[Tex]()\n ts.add(mk().t)",
        "var h = mk()\n h.t = mk().t",
    ];
    for body in stores {
        let src = format!("{TEX}{decls}main() {{\n {body}\n}}\n");
        let diags = {
            let (mut p, d) = parse(&src);
            assert!(d.is_empty());
            p.builtin_ranges = std::iter::once(0..src.len() as u32).collect();
            check(&p).1
        };
        let got: Vec<_> = diags.iter().map(|d| (d.message.as_str(), d.help.as_deref())).collect();
        assert_eq!(got, vec![(moved_out, Some("store the whole value in a variable first"))], "in: {body}");
    }
    // A part of a variable keeps its help.
    let src = format!("{TEX}main() {{\n let h = Holder(t: make(1), n: 1)\n let t = h.t\n}}\n");
    let (mut p, _) = parse(&src);
    p.builtin_ranges = std::iter::once(0..src.len() as u32).collect();
    let d = check(&p).1;
    assert_eq!(d[0].help.as_deref(), Some("move the whole variable that holds it"));

    // Reading a part (an argument, a field, a variant that builds a value) is fine.
    let reads = format!(
        "{TEX}enum Slot {{\n Empty\n Full {{ t: Tex }}\n}}\n{decls}show(t: Tex) -> int {{\n t.w\n}}\n\
         main() {{\n print(show(mk().t), take(many()[0]), mk().n, mk().t.w, many().len)\n let s = Slot.Empty\n}}\n"
    );
    assert!(errors(&reads, true).is_empty());
}
