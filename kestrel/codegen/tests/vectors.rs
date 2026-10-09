mod common;

use common::{call0, jit};

const VECTORS: &str = "pure len2(v: vec3) -> float {
    v.x * v.x + v.y * v.y + v.z * v.z
}

pure mk(n: float) -> vec3 {
    vec3(n, n * 2, n * 3)
}

counted(var n: int) -> vec2 {
    n += 1
    vec2(1, 1)
}

t_ops() -> float {
    var p = vec3(1, 2, 3)
    let v = vec3(0.5, 0, -1)
    p += v * 2
    let q = p.xz
    let r = -p + 1.0
    q.x * 100 + q.y * 10 + r.x + r.z
}

t_scalar_left() -> float {
    let w = 2 * vec2(1, 2) - vec2(1, 1) / 2.0
    w.x * 10 + w.y
}

t_operands_once() -> int {
    var n = 0
    let v = counted(var n) + counted(var n)
    n
}

t_swizzle_of_call() -> float {
    mk(1.0).zyx.x
}

t_param() -> float {
    len2(vec3(1, 2, 2))
}

t_compound_scalar() -> float {
    var v = vec2(1, 2)
    v *= 3
    v.x + v.y
}
";

#[test]
fn vector_arithmetic_is_component_wise() {
    let p = jit(VECTORS);
    assert_eq!(call0::<f32>(&p, "t_ops"), 209.0);
    assert_eq!(call0::<f32>(&p, "t_scalar_left"), 18.5);
    assert_eq!(call0::<f32>(&p, "t_compound_scalar"), 9.0);
}

#[test]
fn each_operand_is_evaluated_once() {
    let p = jit(VECTORS);
    assert_eq!(call0::<i32>(&p, "t_operands_once"), 2);
}

#[test]
fn swizzles_and_vector_parameters() {
    let p = jit(VECTORS);
    assert_eq!(call0::<f32>(&p, "t_swizzle_of_call"), 3.0);
    assert_eq!(call0::<f32>(&p, "t_param"), 9.0);
}
