mod common;

use common::jit_with;
use kestrel_codegen::{ExportTy, Options};

const SRC: &str = "pub pure add(a: int, b: float) -> float { float(a) + b }
pub pure sum(xs: [int]) -> int {
    var s = 0
    for x in xs { s += x }
    s
}
pub pure squares(n: int) -> [int] {
    var out = array[int](n)
    for i from 0 to n { out[i] = i * i }
    out
}
pub pure up(v: vec3) -> vec3 { v + vec3(0, 1, 0) }
pub pure big(x: int) -> bool { x > 10 }
hidden() -> int { 1 }
main() {}
";

#[test]
fn exported_functions_are_listed_with_their_layouts() {
    let p = jit_with(SRC, Options::default());
    let exports = p.exports();
    let names: Vec<&str> = exports.iter().map(|(i, _)| i.name.as_str()).collect();
    assert_eq!(names, ["add", "sum", "squares", "up", "big"]);
    let (add, _) = &exports[0];
    assert_eq!(add.params, [ExportTy::Int, ExportTy::Float]);
    assert_eq!((add.arg_offsets.clone(), add.args_size, add.ret_size), (vec![0, 4], 8, 4));
    let (sum, _) = &exports[1];
    assert_eq!(sum.params, [ExportTy::Seq { list: false, elem: Box::new(ExportTy::Int), elem_size: 4 }]);
}

#[test]
fn thunks_call_through_a_byte_buffer() {
    let p = jit_with(SRC, Options::default());
    let exports = p.exports();
    let thunk = |name: &str| exports.iter().find(|(i, _)| i.name == name).unwrap().1;
    // add(2, 0.5)
    let mut args = [0u8; 8];
    args[..4].copy_from_slice(&2i32.to_le_bytes());
    args[4..].copy_from_slice(&0.5f32.to_le_bytes());
    let mut ret = [0u8; 4];
    unsafe { thunk("add")(args.as_ptr(), ret.as_mut_ptr()) };
    assert_eq!(f32::from_le_bytes(ret), 2.5);
    // sum(xs) reads straight from host memory.
    let xs = [1i32, 2, 3, 4];
    let mut args = [0u8; 16];
    args[..8].copy_from_slice(&(xs.as_ptr() as u64).to_le_bytes());
    args[8..].copy_from_slice(&4i64.to_le_bytes());
    let mut ret = [0u8; 4];
    unsafe { thunk("sum")(args.as_ptr(), ret.as_mut_ptr()) };
    assert_eq!(i32::from_le_bytes(ret), 10);
    // squares(4) returns an array header.
    let mut args = [0u8; 8];
    args[..4].copy_from_slice(&4i32.to_le_bytes());
    let mut ret = [0u8; 16];
    unsafe { thunk("squares")(args.as_ptr(), ret.as_mut_ptr()) };
    let ptr = u64::from_le_bytes(ret[..8].try_into().unwrap()) as *const i32;
    let len = i64::from_le_bytes(ret[8..].try_into().unwrap()) as usize;
    assert_eq!(unsafe { std::slice::from_raw_parts(ptr, len) }, &[0, 1, 4, 9]);
    // up(vec3) passes and returns a struct through memory.
    let mut args = [0u8; 16];
    for (i, v) in [1.0f32, 2.0, 3.0].iter().enumerate() {
        args[i * 4..i * 4 + 4].copy_from_slice(&v.to_le_bytes());
    }
    let mut ret = [0u8; 12];
    unsafe { thunk("up")(args.as_ptr(), ret.as_mut_ptr()) };
    let y = f32::from_le_bytes(ret[4..8].try_into().unwrap());
    assert_eq!(y, 3.0);
    // big(11) is a bool: one byte.
    let mut args = [0u8; 8];
    args[..4].copy_from_slice(&11i32.to_le_bytes());
    let mut ret = [0u8; 1];
    unsafe { thunk("big")(args.as_ptr(), ret.as_mut_ptr()) };
    assert_eq!(ret[0], 1);
}

#[test]
fn a_function_type_in_a_signature_is_listed_as_unsupported_for_a_host() {
    let src = "pub apply(f: (int) -> int, x: int) -> int { f(x) }
pub pick() -> (int) -> int { twice }
twice(x: int) -> int { x * 2 }
main() {}
";
    let p = jit_with(src, Options::default());
    let exports = p.exports();
    let apply = &exports.iter().find(|(i, _)| i.name == "apply").expect("compiled and exported").0;
    assert_eq!(apply.params, [ExportTy::Other("a function".into()), ExportTy::Int]);
    let pick = &exports.iter().find(|(i, _)| i.name == "pick").expect("compiled and exported").0;
    assert_eq!(pick.ret, Some(ExportTy::Other("a function".into())));
    // The thunks that make function values are not exports.
    assert!(exports.iter().all(|(i, _)| !i.name.starts_with("__kestrel_fn_")));
}
