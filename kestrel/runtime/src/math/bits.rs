//! The bits Kestrel's math gives, pinned for a table of inputs per function: signed zeros, a
//! subnormal, `1e22` (hard argument reduction), infinities, NaN and the edges of each domain. A
//! change of these bits changes what saved replays and networked games compute, so it must be a
//! decision, never an accident. `tests/programs/det_math.kes` prints the same rows from compiled
//! code, in the JIT and in exes.

use super::*;
use crate::panic::{kestrel_rt_fmod_f32, kestrel_rt_fmod_f64};
use std::f64::consts::FRAC_PI_4;

const INF: f64 = f64::INFINITY;
const NAN: f64 = f64::NAN;

/// Function name, inputs, and the bits of the result. A NaN result promises only that it is a
/// NaN: payloads aren't part of the promise.
#[rustfmt::skip]
const TABLE: &[(&str, &[f64], u64)] = &[
    ("sin", &[0.0], 0x0000000000000000), // 0.0
    ("sin", &[-0.0], 0x8000000000000000), // -0.0
    ("sin", &[1e-300], 0x01A56E1FC2F8F359), // 1e-300
    ("sin", &[5e-324], 0x0000000000000001), // 5e-324
    ("sin", &[0.5], 0x3FDEAEE8744B05F0), // 0.479425538604203
    ("sin", &[1.0], 0x3FEAED548F090CEE), // 0.8414709848078965
    ("sin", &[2.0], 0x3FED18F6EAD1B446), // 0.9092974268256817
    ("sin", &[3.0], 0x3FC210386DB6D55B), // 0.1411200080598672
    ("sin", &[-3.0], 0xBFC210386DB6D55B), // -0.1411200080598672
    ("sin", &[FRAC_PI_4], 0x3FE6A09E667F3BCC), // 0.7071067811865475
    ("sin", &[100.0], 0xBFE03425B78C4DB8), // -0.5063656411097588
    ("sin", &[1000000.0], 0xBFD6664B2568D867), // -0.34999350217129294
    ("sin", &[1e22], 0xBFEB453AB76BF397), // -0.8522008497671888
    ("sin", &[1e300], 0xBFEA2C16B010E385), // -0.8178819121159085
    ("sin", &[INF], 0xFFF8000000000000), // NaN
    ("sin", &[-INF], 0xFFF8000000000000), // NaN
    ("sin", &[NAN], 0x7FF8000000000000), // NaN
    ("cos", &[0.0], 0x3FF0000000000000), // 1.0
    ("cos", &[-0.0], 0x3FF0000000000000), // 1.0
    ("cos", &[1e-300], 0x3FF0000000000000), // 1.0
    ("cos", &[5e-324], 0x3FF0000000000000), // 1.0
    ("cos", &[0.5], 0x3FEC1528065B7D50), // 0.8775825618903728
    ("cos", &[1.0], 0x3FE14A280FB5068C), // 0.5403023058681398
    ("cos", &[2.0], 0xBFDAA22657537205), // -0.4161468365471424
    ("cos", &[3.0], 0xBFEFAE04BE85E5D2), // -0.9899924966004454
    ("cos", &[-3.0], 0xBFEFAE04BE85E5D2), // -0.9899924966004454
    ("cos", &[FRAC_PI_4], 0x3FE6A09E667F3BCD), // 0.7071067811865476
    ("cos", &[100.0], 0x3FEB981DBF665FDF), // 0.8623188722876839
    ("cos", &[1000000.0], 0x3FEDF9DF9906D32C), // 0.9367521275331447
    ("cos", &[1e22], 0x3FE0BE2CEF01C8F4), // 0.523214785395139
    ("cos", &[1e300], 0xBFE2699022ADC4C1), // -0.5753861119575491
    ("cos", &[INF], 0xFFF8000000000000), // NaN
    ("cos", &[-INF], 0xFFF8000000000000), // NaN
    ("cos", &[NAN], 0x7FF8000000000000), // NaN
    ("tan", &[0.0], 0x0000000000000000), // 0.0
    ("tan", &[-0.0], 0x8000000000000000), // -0.0
    ("tan", &[1e-300], 0x01A56E1FC2F8F359), // 1e-300
    ("tan", &[5e-324], 0x0000000000000001), // 5e-324
    ("tan", &[0.5], 0x3FE17B4F5BF3474A), // 0.5463024898437905
    ("tan", &[1.0], 0x3FF8EB245CBEE3A6), // 1.5574077246549023
    ("tan", &[2.0], 0xC0017AF62E0950F8), // -2.185039863261519
    ("tan", &[3.0], 0xBFC23EF71254B86F), // -0.1425465430742778
    ("tan", &[-3.0], 0x3FC23EF71254B86F), // 0.1425465430742778
    ("tan", &[FRAC_PI_4], 0x3FEFFFFFFFFFFFFF), // 0.9999999999999999
    ("tan", &[100.0], 0xBFE2CA74D62B5D38), // -0.5872139151569291
    ("tan", &[1000000.0], 0xBFD7E9768AB734C0), // -0.373624453987599
    ("tan", &[1e22], 0xBFFA0F79C1B6B258), // -1.628778225606899
    ("tan", &[1e300], 0x3FF6BE411F37AC77), // 1.4214488238747245
    ("tan", &[INF], 0xFFF8000000000000), // NaN
    ("tan", &[-INF], 0xFFF8000000000000), // NaN
    ("tan", &[NAN], 0x7FF8000000000000), // NaN
    ("asin", &[-1.0], 0xBFF921FB54442D18), // -1.5707963267948966
    ("asin", &[-0.5], 0xBFE0C152382D7366), // -0.5235987755982989
    ("asin", &[-0.0], 0x8000000000000000), // -0.0
    ("asin", &[0.0], 0x0000000000000000), // 0.0
    ("asin", &[0.3], 0x3FD380159E14F6FF), // 0.3046926540153975
    ("asin", &[0.5], 0x3FE0C152382D7366), // 0.5235987755982989
    ("asin", &[0.99999], 0x3FF90FA9F3695AA5), // 1.5663241871131188
    ("asin", &[1.0], 0x3FF921FB54442D18), // 1.5707963267948966
    ("asin", &[1.5], 0xFFF8000000000000), // NaN
    ("asin", &[NAN], 0x7FF8000000000000), // NaN
    ("acos", &[-1.0], 0x400921FB54442D18), // 3.141592653589793
    ("acos", &[-0.5], 0x4000C152382D7366), // 2.0943951023931957
    ("acos", &[-0.0], 0x3FF921FB54442D18), // 1.5707963267948966
    ("acos", &[0.0], 0x3FF921FB54442D18), // 1.5707963267948966
    ("acos", &[0.3], 0x3FF441F5ECBEEF59), // 1.2661036727794992
    ("acos", &[0.5], 0x3FF0C152382D7366), // 1.0471975511965979
    ("acos", &[0.99999], 0x3F725160DAD27316), // 0.0044721396817777506
    ("acos", &[1.0], 0x0000000000000000), // 0.0
    ("acos", &[1.5], 0xFFF8000000000000), // NaN
    ("acos", &[NAN], 0x7FF8000000000000), // NaN
    ("atan", &[0.0], 0x0000000000000000), // 0.0
    ("atan", &[-0.0], 0x8000000000000000), // -0.0
    ("atan", &[0.5], 0x3FDDAC670561BB4F), // 0.4636476090008061
    ("atan", &[1.0], 0x3FE921FB54442D18), // 0.7853981633974483
    ("atan", &[-1.0], 0xBFE921FB54442D18), // -0.7853981633974483
    ("atan", &[10000000000.0], 0x3FF921FB543D4DE0), // 1.5707963266948965
    ("atan", &[-1e300], 0xBFF921FB54442D18), // -1.5707963267948966
    ("atan", &[INF], 0x3FF921FB54442D18), // 1.5707963267948966
    ("atan", &[NAN], 0x7FF8000000000000), // NaN
    ("atan2", &[0.0, 1.0], 0x0000000000000000), // 0.0
    ("atan2", &[0.0, -1.0], 0x400921FB54442D18), // 3.141592653589793
    ("atan2", &[-0.0, -1.0], 0xC00921FB54442D18), // -3.141592653589793
    ("atan2", &[1.0, 0.0], 0x3FF921FB54442D18), // 1.5707963267948966
    ("atan2", &[-1.0, 0.0], 0xBFF921FB54442D18), // -1.5707963267948966
    ("atan2", &[1.0, 1.0], 0x3FE921FB54442D18), // 0.7853981633974483
    ("atan2", &[-1.0, -1.0], 0xC002D97C7F3321D2), // -2.356194490192345
    ("atan2", &[0.0, 0.0], 0x0000000000000000), // 0.0
    ("atan2", &[-0.0, -0.0], 0xC00921FB54442D18), // -3.141592653589793
    ("atan2", &[INF, INF], 0x3FE921FB54442D18), // 0.7853981633974483
    ("atan2", &[1.0, INF], 0x0000000000000000), // 0.0
    ("atan2", &[NAN, 1.0], 0x7FF8000000000000), // NaN
    ("exp", &[0.0], 0x3FF0000000000000), // 1.0
    ("exp", &[1.0], 0x4005BF0A8B14576A), // 2.7182818284590455
    ("exp", &[-1.0], 0x3FD78B56362CEF38), // 0.36787944117144233
    ("exp", &[0.5], 0x3FFA61298E1E069C), // 1.6487212707001282
    ("exp", &[10.0], 0x40D5829DCF950560), // 22026.465794806718
    ("exp", &[709.78], 0x7FEFE9CE5C4C52B4), // 1.7928227943945155e308
    ("exp", &[710.0], 0x7FF0000000000000), // inf
    ("exp", &[-745.1], 0x0000000000000001), // 5e-324
    ("exp", &[-746.0], 0x0000000000000000), // 0.0
    ("exp", &[INF], 0x7FF0000000000000), // inf
    ("exp", &[-INF], 0x0000000000000000), // 0.0
    ("exp", &[NAN], 0x7FF8000000000000), // NaN
    ("log", &[1.0], 0x0000000000000000), // 0.0
    ("log", &[2.0], 0x3FE62E42FEFA39EF), // 0.6931471805599453
    ("log", &[0.5], 0xBFE62E42FEFA39EF), // -0.6931471805599453
    ("log", &[10.0], 0x40026BB1BBB55516), // 2.302585092994046
    ("log", &[1e-310], 0xC0864E69394D9508), // -713.8013788281542
    ("log", &[1e300], 0x4085963447F87FB5), // 690.7755278982137
    ("log", &[0.0], 0xFFF0000000000000), // -inf
    ("log", &[-0.0], 0xFFF0000000000000), // -inf
    ("log", &[-1.0], 0xFFF8000000000000), // NaN
    ("log", &[INF], 0x7FF0000000000000), // inf
    ("log", &[NAN], 0x7FF8000000000000), // NaN
    ("pow", &[2.0, 0.5], 0x3FF6A09E667F3BCD), // 1.4142135623730951
    ("pow", &[2.0, 10.0], 0x4090000000000000), // 1024.0
    ("pow", &[10.0, -3.0], 0x3F50624DD2F1A9FC), // 0.001
    ("pow", &[-8.0, 1.0 / 3.0], 0xFFF8000000000000), // NaN
    ("pow", &[-8.0, 3.0], 0xC080000000000000), // -512.0
    ("pow", &[0.0, 0.0], 0x3FF0000000000000), // 1.0
    ("pow", &[NAN, 0.0], 0x3FF0000000000000), // 1.0
    ("pow", &[1.0, NAN], 0x3FF0000000000000), // 1.0
    ("pow", &[1.0000001, 10000000.0], 0x4005BF0A790CE6F2), // 2.7182816941320818
    ("pow", &[2.0, 1024.0], 0x7FF0000000000000), // inf
    ("pow", &[2.0, -1075.0], 0x0000000000000000), // 0.0
    ("pow", &[0.5, -1074.0], 0x7FF0000000000000), // inf
    ("fmod", &[7.5, 2.0], 0x3FF8000000000000), // 1.5
    ("fmod", &[-7.5, 2.0], 0xBFF8000000000000), // -1.5
    ("fmod", &[1e300, 3.0], 0x0000000000000000), // 0.0
    ("fmod", &[5e-324, 1.0], 0x0000000000000001), // 5e-324
    ("fmod", &[1.0, 0.0], 0xFFF8000000000000), // NaN
    ("fmod", &[INF, 1.0], 0xFFF8000000000000), // NaN
    ("fmod", &[1.0, INF], 0x3FF0000000000000), // 1.0
    ("fmod", &[-0.0, 1.0], 0x8000000000000000), // -0.0
];

/// `kestrel_rt_<name>` (what compiled code calls) on `xs`.
fn own(name: &str, xs: &[f64]) -> f64 {
    match (name, xs) {
        ("sin", &[x]) => kestrel_rt_sin(x),
        ("cos", &[x]) => kestrel_rt_cos(x),
        ("tan", &[x]) => kestrel_rt_tan(x),
        ("asin", &[x]) => kestrel_rt_asin(x),
        ("acos", &[x]) => kestrel_rt_acos(x),
        ("atan", &[x]) => kestrel_rt_atan(x),
        ("atan2", &[y, x]) => kestrel_rt_atan2(y, x),
        ("exp", &[x]) => kestrel_rt_exp(x),
        ("log", &[x]) => kestrel_rt_log(x),
        ("pow", &[a, b]) => kestrel_rt_pow(a, b),
        ("fmod", &[a, b]) => kestrel_rt_fmod_f64(a, b),
        _ => panic!("no function {name} of {} arguments", xs.len()),
    }
}

/// The platform's function `name` on `xs`: Rust's `f64` methods, which call the C runtime.
fn platform(name: &str, xs: &[f64]) -> f64 {
    match (name, xs) {
        ("sin", &[x]) => x.sin(),
        ("cos", &[x]) => x.cos(),
        ("tan", &[x]) => x.tan(),
        ("asin", &[x]) => x.asin(),
        ("acos", &[x]) => x.acos(),
        ("atan", &[x]) => x.atan(),
        ("atan2", &[y, x]) => y.atan2(x),
        ("exp", &[x]) => x.exp(),
        ("log", &[x]) => x.ln(),
        ("pow", &[a, b]) => a.powf(b),
        ("fmod", &[a, b]) => a % b,
        _ => panic!("no function {name} of {} arguments", xs.len()),
    }
}

/// `x` on a line of integers where neighbouring floats are neighbouring integers and both zeros
/// are 0, so the distance in ulps is a subtraction.
fn ulp_line(x: f64) -> i128 {
    let magnitude = (x.to_bits() & !(1 << 63)) as i128;
    if x.is_sign_negative() {
        -magnitude
    } else {
        magnitude
    }
}

/// `x` as Rust source.
fn source(x: f64) -> String {
    if x.is_nan() {
        "NAN".to_string()
    } else if x == FRAC_PI_4 {
        "FRAC_PI_4".to_string()
    } else if x.is_infinite() {
        if x > 0.0 { "INF" } else { "-INF" }.to_string()
    } else {
        format!("{x:?}")
    }
}

#[test]
fn every_function_gives_its_pinned_bits() {
    let mut wrong = Vec::new();
    for &(name, xs, bits) in TABLE {
        let got = own(name, xs);
        let expected = f64::from_bits(bits);
        let same = if expected.is_nan() { got.is_nan() } else { got.to_bits() == bits };
        if !same {
            wrong.push(format!("{name}{xs:?} = {got:?} ({:#018X}), pinned {expected:?} ({bits:#018X})", got.to_bits()));
        }
    }
    assert!(wrong.is_empty(), "{} of {} rows differ:\n{}", wrong.len(), TABLE.len(), wrong.join("\n"));
}

#[test]
fn pinned_values_are_within_one_ulp_of_the_platform() {
    let mut far = Vec::new();
    for &(name, xs, bits) in TABLE {
        let pinned = f64::from_bits(bits);
        let theirs = platform(name, xs);
        let close = if pinned.is_nan() {
            theirs.is_nan()
        } else if pinned.is_infinite() || pinned == 0.0 {
            theirs.to_bits() == pinned.to_bits()
        } else {
            theirs.is_finite() && (ulp_line(pinned) - ulp_line(theirs)).abs() <= 1
        };
        if !close {
            far.push(format!("{name}{xs:?}: pinned {pinned:?}, platform {theirs:?}"));
        }
    }
    assert!(far.is_empty(), "pinned values more than 1 ulp from the platform's:\n{}", far.join("\n"));
}

#[test]
fn fmod_f32_matches_f64_for_exact_cases() {
    assert_eq!(kestrel_rt_fmod_f32(7.5, 2.0), 1.5);
    assert_eq!(kestrel_rt_fmod_f32(-7.5, 2.0), -1.5);
    assert!(kestrel_rt_fmod_f32(1.0, 0.0).is_nan());
}

/// The `float` inputs `tests/programs/det_math.kes` prints `sin`, `cos`, `atan2(x, 0.75)`, `exp`,
/// `log` and `pow(1.25, x)` of (the wide functions: the `float` is passed as it is, computed in `f64`
/// and rounded once).
const FLOAT_INPUTS: [f32; 6] = [0.5, 1.0, 2.5, -3.0, 100.0, 1234.5];

/// Prints `TABLE` with today's results, to paste over it when the pinned bits change on purpose,
/// then the `// expect:` lines of `tests/programs/det_math.kes`, which must change with it.
#[test]
#[ignore = "prints the table's source: run it by hand with --nocapture"]
fn print_table() {
    for &(name, xs, _) in TABLE {
        let got = own(name, xs);
        let inputs: Vec<String> = xs.iter().map(|&x| source(x)).collect();
        println!("    (\"{name}\", &[{}], {:#018X}), // {got:?}", inputs.join(", "), got.to_bits());
    }
    println!();
    for &(name, xs, _) in TABLE {
        println!("// expect: {}", own(name, xs));
    }
    for x in FLOAT_INPUTS {
        let row = [
            kestrel_rt_sinf(x),
            kestrel_rt_cosf(x),
            kestrel_rt_atan2f(x, 0.75),
            kestrel_rt_expf(x),
            kestrel_rt_logf(x),
            kestrel_rt_powf(1.25, x),
        ];
        let row: Vec<String> = row.iter().map(|r| r.to_string()).collect();
        println!("// expect: {}", row.join(" "));
    }
}
