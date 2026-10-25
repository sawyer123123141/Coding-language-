//! Hover and go to definition on the analysed program: what `resolve` finds at a place.

use std::path::Path;

use kestrel_driver::{analyze, Analysis};
use kestrel_lsp::resolve::{resolve, Symbol};
use kestrel_syntax::Span;

const MAIN: &str = r#"use geom
use { Shape, scale from geom }

let LIMIT = 10

struct Point {
    x: float
    y: float

    pure len(self) -> float {
        sqrt(self.x * self.x + self.y * self.y)
    }
}

enum Hit {
    Miss
    Struck { damage: int }
}

pure clamp_damage(d: int) -> int {
    min(d, LIMIT)
}

main() {
    var total = 0
    let p = Point(x: 3, y: 4)
    let n: float = p.len()
    let h = Hit.Struck(damage: 12)
    match h {
        Struck s { total += clamp_damage(s.damage) }
        Miss { }
    }
    for i from 0 to 3 {
        total += i
    }
    let a = geom.area(2, 3)
    let s = scale(2)
    let c = Shape.Circle(r: 1.5)
    print("total {total} {p.x}", n, a, s, c)
}
"#;

const GEOM: &str = "pub area(w: int, h: int) -> int {
    w * h
}

pub scale(k: int) -> int {
    k * 2
}

pub enum Shape {
    Circle { r: float }
    Dot
}
";

fn analysed() -> Analysis {
    let read = |p: &Path| (p.file_name()? == "geom.kes").then(|| GEOM.to_string());
    let a = analyze("game/main.kes", MAIN, true, &read);
    assert!(a.diags.is_empty(), "{:?}", a.diags);
    a
}

/// The offset of the `nth` (from 0) `needle` in `MAIN`, plus `into` bytes.
fn at(needle: &str, nth: usize, into: u32) -> u32 {
    let start = MAIN.match_indices(needle).nth(nth).unwrap_or_else(|| panic!("no {needle} #{nth}")).0;
    start as u32 + into
}

fn sym(needle: &str, nth: usize) -> Symbol {
    let a = analysed();
    resolve(&a, at(needle, nth, 1)).unwrap_or_else(|| panic!("nothing at {needle} #{nth}"))
}

/// The text a definition span covers, and which source it is in.
fn defined(def: Span) -> (usize, String) {
    let a = analysed();
    let i = a.source_at(def.start);
    let s = &a.sources[i];
    (i, s.src[(def.start - s.base) as usize..(def.end - s.base) as usize].to_string())
}

#[test]
fn a_local_shows_its_type_and_where_it_is_declared() {
    let s = sym("total", 1);
    assert_eq!(s.code, "var total: int");
    assert_eq!(s.def.map(|d| d.start), Some(at("total", 0, 0)));
    // In an interpolated string too.
    let inside = at("{total}", 0, 2);
    assert_eq!(resolve(&analysed(), inside).unwrap().code, "var total: int");
    assert_eq!(sym("i from", 0).code, "let i: int");
    assert_eq!(sym("n: float", 0).code, "let n: float");
}

#[test]
fn a_function_shows_its_signature() {
    let s = sym("clamp_damage(s", 0);
    assert_eq!(s.code, "pure clamp_damage(d: int) -> int");
    assert_eq!(defined(s.def.unwrap()), (0, "clamp_damage".to_string()));
    // The declaration's own name, and a built-in function.
    assert_eq!(sym("clamp_damage(d", 0).code, "pure clamp_damage(d: int) -> int");
    assert!(sym("min(", 0).code.starts_with("min"), "{:?}", sym("min(", 0));
}

#[test]
fn structs_fields_and_methods() {
    let s = sym("Point(x", 0);
    assert_eq!(s.code, "struct Point {\n    x: float\n    y: float\n}");
    assert_eq!(defined(s.def.unwrap()).1, "Point");
    let len = sym("len()", 0);
    assert_eq!(len.code, "pure Point.len(self) -> float");
    assert_eq!(defined(len.def.unwrap()).1, "len");
    // `p.x` in the string: the field.
    let x = resolve(&analysed(), at("p.x}", 0, 2)).unwrap();
    assert_eq!(x.code, "Point.x: float");
    assert_eq!(x.def.map(|d| d.start), Some(at("x: float", 0, 0)));
    // A field's declaration.
    assert_eq!(sym("y: float", 0).code, "Point.y: float");
}

#[test]
fn enums_variants_and_match_arms() {
    let v = sym("Struck(damage", 0);
    assert_eq!(v.code, "Hit.Struck(damage: int)");
    assert_eq!(defined(v.def.unwrap()).1, "Struck");
    // The arm's variant name, and the arm's binding.
    assert_eq!(sym("Struck s", 0).code, "Hit.Struck(damage: int)");
    assert_eq!(sym("s.damage", 0).code, "let s: Hit.Struck");
    let field = resolve(&analysed(), at("s.damage", 0, 3)).unwrap();
    assert_eq!(field.code, "Hit.Struck.damage: int");
    assert_eq!(defined(field.def.unwrap()).1, "damage");
}

#[test]
fn a_constant_shows_its_type() {
    let c = sym("LIMIT)", 0);
    assert_eq!(c.code, "let LIMIT: int");
    assert_eq!(defined(c.def.unwrap()), (0, "LIMIT".to_string()));
}

#[test]
fn items_of_another_module_lead_to_its_file() {
    let area = sym("area(2", 0);
    assert_eq!(area.code, "geom.area(w: int, h: int) -> int");
    assert_eq!(defined(area.def.unwrap()), (1, "area".to_string()));
    // On `geom` of `geom.area` too: the whole `geom.area` names the function.
    assert_eq!(sym("geom.area", 0).code, area.code);
    let scale = sym("scale(2", 0);
    assert_eq!(defined(scale.def.unwrap()), (1, "scale".to_string()));
    let circle = sym("Circle(r", 0);
    assert_eq!(circle.code, "geom.Shape.Circle(r: float)");
    assert_eq!(defined(circle.def.unwrap()), (1, "Circle".to_string()));
}

#[test]
fn the_names_of_a_use_line_lead_to_the_module() {
    let m = sym("geom\n", 0);
    assert_eq!(m.code, "module geom");
    assert_eq!(analysed().source_at(m.def.unwrap().start), 1);
    let scale = sym("scale from", 0);
    assert_eq!(defined(scale.def.unwrap()), (1, "scale".to_string()));
    let shape = sym("Shape, scale", 0);
    assert!(shape.code.starts_with("enum geom.Shape {"), "{}", shape.code);
}

#[test]
fn types_in_annotations() {
    let a = analysed();
    let f = resolve(&a, at("n: float", 0, 4)).unwrap();
    assert_eq!(f.code, "float");
    assert_eq!(f.def, None);
}

#[test]
fn hover_works_with_check_errors_elsewhere() {
    let src = "main() {\n    let bad: int = true\n    let fine = 2.5\n    print(fine)\n}\n";
    let a = analyze("main.kes", src, true, &|_| None);
    assert_eq!(a.diags.len(), 1);
    let off = src.rfind("fine").unwrap() as u32 + 1;
    assert_eq!(resolve(&a, off).unwrap().code, "let fine: float");
}
