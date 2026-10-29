//! The bits Kestrel's `float` math gives (the wide functions of `wide.rs`), pinned for a table of
//! inputs per function: signed zeros, a subnormal, the tiny bound, the fast-range bound, infinities,
//! NaN and the edges of each domain. Like `bits.rs`, a change of these bits changes what saved
//! replays and networked games compute, so it must be a decision. `tests/programs/det_math.kes`
//! prints the same rows from compiled code.

// The inputs are `f32` roundings of constants like π/4 and sqrt(2), on purpose.
#![allow(clippy::approx_constant)]

use super::sweep;

const INF: f32 = f32::INFINITY;
const NAN: f32 = f32::NAN;

/// Function name, inputs (`y, x` for `atan2`, `a, b` for `pow` and `fmod`), and the bits of the
/// result. A NaN result promises only that it is a NaN.
#[rustfmt::skip]
const FLOAT_TABLE: &[(&str, &[f32], u32)] = &[
    ("sin", &[0.0], 0x00000000), // 0.0
    ("sin", &[-0.0], 0x80000000), // -0.0
    ("sin", &[1e-40], 0x000116C2), // 1e-40
    ("sin", &[0.00024414063], 0x39800000), // 0.00024414063
    ("sin", &[-0.00024414063], 0xB9800000), // -0.00024414063
    ("sin", &[0.5], 0x3EF57744), // 0.47942555
    ("sin", &[1.0], 0x3F576AA4), // 0.84147096
    ("sin", &[2.5], 0x3F193578), // 0.5984721
    ("sin", &[-3.0], 0xBE1081C3), // -0.14112
    ("sin", &[0.7853982], 0x3F3504F3), // 0.70710677
    ("sin", &[1.5707964], 0x3F800000), // 1.0
    ("sin", &[100.0], 0xBF01A12E), // -0.50636566
    ("sin", &[1234.5], 0x3E14E299), // 0.14539565
    ("sin", &[399999970.0], 0x3F491743), // 0.7855112
    ("sin", &[400000000.0], 0x3F7F1D4C), // 0.9965408
    ("sin", &[1e30], 0xBF4A89B0), // -0.79116344
    ("sin", &[INF], 0xFFC00000), // NaN
    ("sin", &[-INF], 0xFFC00000), // NaN
    ("sin", &[NAN], 0x7FC00000), // NaN
    ("cos", &[0.0], 0x3F800000), // 1.0
    ("cos", &[-0.0], 0x3F800000), // 1.0
    ("cos", &[1e-40], 0x3F800000), // 1.0
    ("cos", &[0.00024414063], 0x3F800000), // 1.0
    ("cos", &[-0.00024414063], 0x3F800000), // 1.0
    ("cos", &[0.5], 0x3F60A940), // 0.87758255
    ("cos", &[1.0], 0x3F0A5140), // 0.5403023
    ("cos", &[2.5], 0xBF4D17BF), // -0.8011436
    ("cos", &[-3.0], 0xBF7D7026), // -0.9899925
    ("cos", &[0.7853982], 0x3F3504F3), // 0.70710677
    ("cos", &[1.5707964], 0xB33BBD2E), // -4.371139e-8
    ("cos", &[100.0], 0x3F5CC0EE), // 0.8623189
    ("cos", &[1234.5], 0xBF7D4796), // -0.98937356
    ("cos", &[399999970.0], 0x3F1E6CC9), // 0.6188474
    ("cos", &[400000000.0], 0x3DAA3307), // 0.08310514
    ("cos", &[1e30], 0xBF1C9222), // -0.6116048
    ("cos", &[INF], 0xFFC00000), // NaN
    ("cos", &[-INF], 0xFFC00000), // NaN
    ("cos", &[NAN], 0x7FC00000), // NaN
    ("tan", &[0.0], 0x00000000), // 0.0
    ("tan", &[-0.0], 0x80000000), // -0.0
    ("tan", &[1e-40], 0x000116C2), // 1e-40
    ("tan", &[0.00024414063], 0x39800000), // 0.00024414063
    ("tan", &[-0.00024414063], 0xB9800000), // -0.00024414063
    ("tan", &[0.5], 0x3F0BDA7B), // 0.5463025
    ("tan", &[1.0], 0x3FC75923), // 1.5574077
    ("tan", &[2.5], 0xBF3F3CDA), // -0.7470223
    ("tan", &[-3.0], 0x3E11F7B9), // 0.14254655
    ("tan", &[0.7853982], 0x3F800000), // 1.0
    ("tan", &[1.5707964], 0xCBAE8A4A), // -22877332.0
    ("tan", &[100.0], 0xBF1653A7), // -0.58721393
    ("tan", &[1234.5], 0xBE167BF8), // -0.14695728
    ("tan", &[399999970.0], 0x3FA278DB), // 1.2693132
    ("tan", &[400000000.0], 0x413FDC79), // 11.991326
    ("tan", &[1e30], 0x3FA5943B), // 1.2935861
    ("tan", &[INF], 0xFFC00000), // NaN
    ("tan", &[-INF], 0xFFC00000), // NaN
    ("tan", &[NAN], 0x7FC00000), // NaN
    ("asin", &[-1.0], 0xBFC90FDB), // -1.5707964
    ("asin", &[-0.5], 0xBF060A92), // -0.5235988
    ("asin", &[-0.0], 0x80000000), // -0.0
    ("asin", &[0.0], 0x00000000), // 0.0
    ("asin", &[0.3], 0x3E9C00AD), // 0.30469266
    ("asin", &[0.5], 0x3F060A92), // 0.5235988
    ("asin", &[0.70710677], 0x3F490FDA), // 0.7853981
    ("asin", &[0.99999994], 0x3FC9048A), // 1.570451
    ("asin", &[1.0], 0x3FC90FDB), // 1.5707964
    ("asin", &[1.0000001], 0xFFC00000), // NaN
    ("asin", &[NAN], 0x7FC00000), // NaN
    ("acos", &[-1.0], 0x40490FDB), // 3.1415927
    ("acos", &[-0.5], 0x40060A92), // 2.0943952
    ("acos", &[-0.0], 0x3FC90FDB), // 1.5707964
    ("acos", &[0.0], 0x3FC90FDB), // 1.5707964
    ("acos", &[0.3], 0x3FA20FAF), // 1.2661036
    ("acos", &[0.5], 0x3F860A92), // 1.0471976
    ("acos", &[0.70710677], 0x3F490FDB), // 0.7853982
    ("acos", &[0.99999994], 0x39B504F3), // 0.00034526698
    ("acos", &[1.0], 0x00000000), // 0.0
    ("acos", &[1.0000001], 0xFFC00000), // NaN
    ("acos", &[NAN], 0x7FC00000), // NaN
    ("atan", &[0.0], 0x00000000), // 0.0
    ("atan", &[-0.0], 0x80000000), // -0.0
    ("atan", &[0.5], 0x3EED6338), // 0.4636476
    ("atan", &[1.0], 0x3F490FDB), // 0.7853982
    ("atan", &[-1.0], 0xBF490FDB), // -0.7853982
    ("atan", &[0.41421357], 0x3EC90FDB), // 0.3926991
    ("atan", &[10000000000.0], 0x3FC90FDB), // 1.5707964
    ("atan", &[-1e30], 0xBFC90FDB), // -1.5707964
    ("atan", &[INF], 0x3FC90FDB), // 1.5707964
    ("atan", &[-INF], 0xBFC90FDB), // -1.5707964
    ("atan", &[NAN], 0x7FC00000), // NaN
    ("atan2", &[0.0, 1.0], 0x00000000), // 0.0
    ("atan2", &[0.0, -1.0], 0x40490FDB), // 3.1415927
    ("atan2", &[-0.0, -1.0], 0xC0490FDB), // -3.1415927
    ("atan2", &[1.0, 0.0], 0x3FC90FDB), // 1.5707964
    ("atan2", &[-1.0, 0.0], 0xBFC90FDB), // -1.5707964
    ("atan2", &[1.0, -0.0], 0x3FC90FDB), // 1.5707964
    ("atan2", &[1.0, 1.0], 0x3F490FDB), // 0.7853982
    ("atan2", &[-1.0, -1.0], 0xC016CBE4), // -2.3561945
    ("atan2", &[0.0, 0.0], 0x00000000), // 0.0
    ("atan2", &[-0.0, -0.0], 0xC0490FDB), // -3.1415927
    ("atan2", &[INF, INF], 0x3F490FDB), // 0.7853982
    ("atan2", &[1.0, INF], 0x00000000), // 0.0
    ("atan2", &[NAN, 1.0], 0x7FC00000), // NaN
    ("atan2", &[0.41421357, 1.0], 0x3EC90FDB), // 0.3926991
    ("atan2", &[3.0, 0.75], 0x3FA9B465), // 1.3258177
    ("exp", &[0.0], 0x3F800000), // 1.0
    ("exp", &[-0.0], 0x3F800000), // 1.0
    ("exp", &[1.0], 0x402DF854), // 2.7182817
    ("exp", &[-1.0], 0x3EBC5AB2), // 0.36787945
    ("exp", &[0.5], 0x3FD3094C), // 1.6487212
    ("exp", &[10.0], 0x46AC14EE), // 22026.465
    ("exp", &[88.72], 0x7F7F4648), // 3.3931806e38
    ("exp", &[88.73], 0x7F800000), // inf
    ("exp", &[89.0], 0x7F800000), // inf
    ("exp", &[100.0], 0x7F800000), // inf
    ("exp", &[-87.0], 0x00B33687), // 1.6458115e-38
    ("exp", &[-103.97], 0x00000001), // 1e-45
    ("exp", &[-104.0], 0x00000000), // 0.0
    ("exp", &[-110.0], 0x00000000), // 0.0
    ("exp", &[1e-40], 0x3F800000), // 1.0
    ("exp", &[INF], 0x7F800000), // inf
    ("exp", &[-INF], 0x00000000), // 0.0
    ("exp", &[NAN], 0x7FC00000), // NaN
    ("log", &[1.0], 0x00000000), // 0.0
    ("log", &[2.0], 0x3F317218), // 0.6931472
    ("log", &[0.5], 0xBF317218), // -0.6931472
    ("log", &[10.0], 0x40135D8E), // 2.3025851
    ("log", &[0.70710677], 0xBEB17219), // -0.34657362
    ("log", &[1.4142135], 0x3EB17217), // 0.34657356
    ("log", &[1e-40], 0xC2B834F2), // -92.10341
    ("log", &[1e-45], 0xC2CE8ED0), // -103.27893
    ("log", &[3.4028235e38], 0x42B17218), // 88.72284
    ("log", &[0.0], 0xFF800000), // -inf
    ("log", &[-0.0], 0xFF800000), // -inf
    ("log", &[-1.0], 0xFFC00000), // NaN
    ("log", &[INF], 0x7F800000), // inf
    ("log", &[NAN], 0x7FC00000), // NaN
    ("pow", &[2.0, 0.5], 0x3FB504F3), // 1.4142135
    ("pow", &[2.0, 10.0], 0x44800000), // 1024.0
    ("pow", &[10.0, -3.0], 0x3A83126F), // 0.001
    ("pow", &[-8.0, 3.0], 0xC4000000), // -512.0
    ("pow", &[-8.0, 0.33333334], 0xFFC00000), // NaN
    ("pow", &[0.0, 0.0], 0x3F800000), // 1.0
    ("pow", &[NAN, 0.0], 0x3F800000), // 1.0
    ("pow", &[1.0, NAN], 0x3F800000), // 1.0
    ("pow", &[1.0000001, 10000000.0], 0x4052D05E), // 3.2939677
    ("pow", &[2.0, 128.0], 0x7F800000), // inf
    ("pow", &[2.0, -150.0], 0x00000001), // 1e-45
    ("pow", &[0.5, -149.0], 0x7F800000), // inf
    ("pow", &[1.25, 2.5], 0x3FDF9B57), // 1.7469281
    ("fmod", &[7.5, 2.0], 0x3FC00000), // 1.5
    ("fmod", &[-7.5, 2.0], 0xBFC00000), // -1.5
    ("fmod", &[1e30, 3.0], 0x00000000), // 0.0
    ("fmod", &[1e-40, 1.0], 0x000116C2), // 1e-40
    ("fmod", &[1.0, 0.0], 0xFFC00000), // NaN
    ("fmod", &[INF, 1.0], 0xFFC00000), // NaN
    ("fmod", &[1.0, INF], 0x3F800000), // 1.0
    ("fmod", &[-0.0, 1.0], 0x80000000), // -0.0
    ("fmod", &[5.0, 2.5], 0x00000000), // 0.0
    ("fmod", &[-5.0, 2.5], 0x80000000), // -0.0
    ("fmod", &[1e-40, 3e-41], 0x00001BDF), // 9.998e-42
    ("fmod", &[536870900.0, 1.0], 0x00000000), // 0.0
    ("fmod", &[3.0, -2.0], 0x3F800000), // 1.0
];

/// The row's two arguments (the second is 0 for one-argument functions).
fn args(xs: &[f32]) -> (f32, f32) {
    match *xs {
        [a] => (a, 0.0),
        [a, b] => (a, b),
        _ => panic!("a row has one or two inputs"),
    }
}

/// `kestrel_rt_<name>f` (what compiled code calls) on the row's inputs.
fn own(name: &str, xs: &[f32]) -> f32 {
    let (a, b) = args(xs);
    sweep::wide(name, a, b)
}

/// `x` as Rust source.
fn source(x: f32) -> String {
    if x.is_nan() {
        "NAN".to_string()
    } else if x.is_infinite() {
        if x > 0.0 { "INF" } else { "-INF" }.to_string()
    } else {
        format!("{x:?}")
    }
}

#[test]
fn every_float_function_gives_its_pinned_bits() {
    let mut wrong = Vec::new();
    for &(name, xs, bits) in FLOAT_TABLE {
        let got = own(name, xs);
        let expected = f32::from_bits(bits);
        let same = if expected.is_nan() { got.is_nan() } else { got.to_bits() == bits };
        if !same {
            wrong.push(format!("{name}{xs:?} = {got:?} ({:#010X}), pinned {expected:?} ({bits:#010X})", got.to_bits()));
        }
    }
    assert!(wrong.is_empty(), "{} of {} rows differ:\n{}", wrong.len(), FLOAT_TABLE.len(), wrong.join("\n"));
}

#[test]
fn pinned_float_values_are_within_0_51_ulp() {
    let mut far = Vec::new();
    for &(name, xs, bits) in FLOAT_TABLE {
        let (a, b) = args(xs);
        let u = sweep::ulps(f32::from_bits(bits), sweep::reference(name, a, b));
        if u > super::wide::MAX_ULP {
            far.push(format!("{name}{xs:?}: pinned {:?} is {u} ulp from {:?}", f32::from_bits(bits), sweep::reference(name, a, b)));
        }
    }
    assert!(far.is_empty(), "pinned values more than 0.51 ulp from libm's f64 result:\n{}", far.join("\n"));
}

#[test]
fn every_pinned_input_is_a_sweep_edge() {
    let edges = sweep::edges();
    let missing: Vec<String> = FLOAT_TABLE
        .iter()
        .flat_map(|&(name, xs, _)| xs.iter().map(move |&x| (name, x)))
        .filter(|&(_, x)| !edges.iter().any(|&e| sweep::same(e, x)))
        .map(|(name, x)| format!("{name}: {x:e}"))
        .collect();
    assert!(missing.is_empty(), "inputs of FLOAT_TABLE missing from sweep::edges:\n{}", missing.join("\n"));
}

/// Prints `FLOAT_TABLE` with today's results, to paste over it when the pinned bits change on
/// purpose, then the `// expect:` lines of `tests/programs/det_math.kes`, which change with it.
#[test]
#[ignore = "prints the table's source: run it by hand with --nocapture"]
fn print_float_table() {
    for &(name, xs, _) in FLOAT_TABLE {
        let got = own(name, xs);
        let inputs: Vec<String> = xs.iter().map(|&x| source(x)).collect();
        println!("    (\"{name}\", &[{}], {:#010X}), // {got:?}", inputs.join(", "), got.to_bits());
    }
    println!();
    for &(name, xs, _) in FLOAT_TABLE {
        println!("// expect: {}", own(name, xs));
    }
}
