mod common;

use common::{call0, call1, jit};

const STRUCTS: &str = "struct Point {
    x: int
    y: int = 10

    pure sum(self) -> int { self.x + self.y }
    shift(var self, by: int) {
        self.x += by
    }
    swap(var self) {
        self = Point(self.y, self.x)
    }
}

struct Line {
    a: Point
    b: Point
}

struct Mixed {
    a: u8
    b: i64
    c: u8
}

pure mid(l: Line) -> Point {
    Point((l.a.x + l.b.x) / 2, (l.a.y + l.b.y) / 2)
}

pure pass(p: Point) -> Point { p }

bump(var n: int) {
    n += 1
}

t_default() -> int {
    let p = Point(3)
    p.x * 100 + p.y
}

t_methods() -> int {
    var p = Point(1, 2)
    p.shift(5)
    p.shift(by: p.x)
    p.sum()
}

t_swap() -> int {
    var p = Point(1, 2)
    p.swap()
    p.x * 10 + p.y
}

t_copies() -> int {
    var l = Line(Point(1, 2), Point(3, 4))
    var c = l
    c.a.x = 100
    l.b.y += 1
    l.a.x * 1000 + c.a.x + l.b.y
}

t_return() -> int {
    let m = mid(Line(Point(2, 4), Point(6, 8)))
    m.x * 10 + m.y
}

t_field_of_call() -> int {
    mid(Line(Point(0, 0), Point(10, 20))).y
}

t_var_field() -> int {
    var l = Line(Point(1, 1), Point(1, 1))
    bump(var l.b.x)
    bump(var l.b.x)
    l.b.x
}

t_if(n: int) -> int {
    let p = if n > 0 { Point(1, 2) } else { Point(3, 4) }
    p.sum()
}

t_pass_copies() -> int {
    var a = Point(5, 6)
    let b = pass(a)
    a.x = 0
    b.x
}

t_mixed() -> i64 {
    let m = Mixed(1, 2000000000000, 3)
    i64(m.a) + m.b + i64(m.c)
}

t_labels() -> int {
    let p = Point(y: 7, x: 2)
    p.x * 10 + p.y
}
";

#[test]
fn structs_fields_and_defaults() {
    let p = jit(STRUCTS);
    assert_eq!(call0::<i32>(&p, "t_default"), 310);
    assert_eq!(call0::<i32>(&p, "t_labels"), 27);
    assert_eq!(call0::<i64>(&p, "t_mixed"), 2_000_000_000_004);
}

#[test]
fn methods_change_their_receiver() {
    let p = jit(STRUCTS);
    assert_eq!(call0::<i32>(&p, "t_methods"), 14);
    assert_eq!(call0::<i32>(&p, "t_swap"), 21);
}

#[test]
fn struct_copies_are_independent() {
    let p = jit(STRUCTS);
    assert_eq!(call0::<i32>(&p, "t_copies"), 1105);
    assert_eq!(call0::<i32>(&p, "t_pass_copies"), 5);
}

#[test]
fn structs_pass_and_return() {
    let p = jit(STRUCTS);
    assert_eq!(call0::<i32>(&p, "t_return"), 46);
    assert_eq!(call0::<i32>(&p, "t_field_of_call"), 10);
    assert_eq!(call0::<i32>(&p, "t_var_field"), 3);
    assert_eq!(call1(&p, "t_if", 1), 3);
    assert_eq!(call1(&p, "t_if", -1), 7);
}
