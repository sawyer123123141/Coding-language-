//! The step after `compile` for annotated programs (`prove`): a program without annotations is left
//! exactly as `compile` made it; an annotated one is analysed once, in place, for its build.

use kestrel_driver::{compile, prove, Compiled, DiagStyle};
use kestrel_kir::range::Arith;

fn compiled(src: &str) -> Compiled {
    let (c, text) = compile("main.kes", src, DiagStyle::Short);
    c.unwrap_or_else(|| panic!("{text}"))
}

#[test]
fn a_program_without_annotations_is_left_alone() {
    let src = "pure f(xs: [int], i: int) -> int { xs[i] + 1 }\nmain() { print(f([1, 2], 1)) }\n";
    let mut c = compiled(src);
    let before = c.module.clone();
    let proved = prove(&mut c, true, true, DiagStyle::Short);
    assert!(proved.ok);
    assert_eq!(proved.text, "");
    assert!(proved.analysis.is_none());
    assert_eq!(c.module.analysed, None);
    assert_eq!(c.module, before);
}

#[test]
fn an_annotated_program_is_analysed_once_in_place() {
    let src = "@prove(no_alloc)\npure f(x: int) -> int { (x & 255) * 1000 }\nmain() { print(f(3)) }\n";
    for (checked, mode) in [(true, Arith::Checked), (false, Arith::Wrapping)] {
        let mut c = compiled(src);
        let proved = prove(&mut c, checked, false, DiagStyle::Short);
        assert!(proved.ok, "{}", proved.text);
        assert_eq!(c.module.analysed, Some(mode));
        let analysis = proved.analysis.expect("the analysis");
        // Marked in place: in a checked build the multiply's overflow check is proven.
        let f = c.module.funcs.iter().position(|f| f.name == "f").unwrap();
        assert_eq!(format!("{:?}", c.module.funcs[f].body).contains("Proven"), checked);
        assert_eq!(analysis.funcs.len(), c.module.funcs.len());
    }
}

#[test]
fn an_assumption_alone_is_analysed_only_when_warnings_are_wanted() {
    let src = "show(n: int) {\n    @assume(n > 0)\n    print(n)\n}\nmain() { show(1) }\n";
    let mut c = compiled(src);
    let proved = prove(&mut c, true, false, DiagStyle::Short);
    assert!(proved.ok && proved.text.is_empty() && proved.analysis.is_none());
    assert_eq!(c.module.analysed, None);
    let proved = prove(&mut c, true, true, DiagStyle::Short);
    assert!(proved.ok);
    assert_eq!(proved.text, "2:5: warning: `@assume(n > 0)` removes no check; delete it\n");
    assert_eq!(c.module.analysed, Some(Arith::Checked));
}

const PICKS: &str = "interface Pick {
    pure pick(self, xs: list[int], i: int) -> int
}
struct A: Pick {
    n: int
    pure pick(self, xs: list[int], i: int) -> int { xs[i] }
}
struct B: Pick {
    n: int
    pure pick(self, xs: list[int], i: int) -> int { 0 }
}
pure take[T: Pick](x: T, xs: list[int], i: int) -> int {
    @assume(i >= 0 and i < xs.len)
    x.pick(xs, i)
}
";

#[test]
fn an_assumption_one_copy_needs_is_kept() {
    // Each copy of `take` lowers its own assumption: `take[A]`'s removes the bounds check of the
    // index it inlines, `take[B]`'s removes none. The warning is for the `@assume`, so it is given
    // only when no copy needs it (P32).
    let both = format!("{PICKS}main() {{\n    let xs: list[int] = [1, 2]\n    print(take(A(n: 1), xs, 0), take(B(n: 1), xs, 0))\n}}\n");
    let mut c = compiled(&both);
    let proved = prove(&mut c, true, true, DiagStyle::Short);
    assert!(proved.ok, "{}", proved.text);
    assert!(!proved.text.contains("removes no check"), "{}", proved.text);
    let only_b = format!("{PICKS}main() {{\n    let xs: list[int] = [1, 2]\n    print(take(B(n: 1), xs, 0))\n}}\n");
    let mut c = compiled(&only_b);
    let proved = prove(&mut c, true, true, DiagStyle::Short);
    assert_eq!(proved.text.matches("removes no check").count(), 1, "{}", proved.text);
}
