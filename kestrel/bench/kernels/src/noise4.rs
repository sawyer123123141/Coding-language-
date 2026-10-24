//! The value noise of `noise_scalar.kes`, `noise_corners.kes` and `noise_wide.kes` in Rust:
//! `bench_scalar` (one sample at a time, plain `i32`/`f32`) and `bench_wide` (four samples at once
//! with SSE4.1 intrinsics, as `noise_wide.kes` does with `ivec4`/`vec4`). The same arithmetic as
//! the Kestrel kernels: wrapping `i32` multiplies, arithmetic `>>`, `f32` maths, the sum in `f64`
//! in sample order, so every version gives the same checksum.

use std::arch::x86_64::*;

const SEED: i32 = 1337;

#[inline(always)]
fn hash(x: i32, y: i32, seed: i32) -> i32 {
    let mut h = x.wrapping_mul(0x27D4_EB2D) ^ y.wrapping_mul(0x1656_67B1) ^ seed.wrapping_mul(0x5BD1_E995);
    h = (h ^ (h >> 15)).wrapping_mul(0x2C1B_3C6D);
    h = (h ^ (h >> 12)).wrapping_mul(0x297A_2D39);
    h ^ (h >> 15)
}

#[inline(always)]
fn lattice(h: i32) -> f32 {
    (h & 0xFFFF) as f32 / 65535.0
}

#[inline(always)]
fn smooth(t: f32) -> f32 {
    t * t * (3.0 - 2.0 * t)
}

#[inline(always)]
fn noise(x: i32, y: i32) -> f32 {
    let (cx, cy) = (x >> 4, y >> 4);
    let fx = smooth((x & 15) as f32 / 16.0);
    let fy = smooth((y & 15) as f32 / 16.0);
    let a = lattice(hash(cx, cy, SEED));
    let b = lattice(hash(cx + 1, cy, SEED));
    let c = lattice(hash(cx, cy + 1, SEED));
    let d = lattice(hash(cx + 1, cy + 1, SEED));
    let top = a + (b - a) * fx;
    let bottom = c + (d - c) * fx;
    top + (bottom - top) * fy
}

/// `n * iters` samples, one at a time.
#[inline(never)]
pub fn bench_scalar(n: i32, iters: i32) -> f64 {
    let mut sum = 0.0f64;
    for k in 0..iters {
        for i in 0..n {
            sum += noise(i & 1023, (i >> 10) + k) as f64;
        }
    }
    sum
}

#[inline]
#[target_feature(enable = "sse4.1")]
unsafe fn hash4(x: __m128i, y: __m128i, seed: i32) -> __m128i {
    let s = _mm_set1_epi32(seed.wrapping_mul(0x5BD1_E995));
    let mut h = _mm_xor_si128(
        _mm_xor_si128(_mm_mullo_epi32(x, _mm_set1_epi32(0x27D4_EB2D)), _mm_mullo_epi32(y, _mm_set1_epi32(0x1656_67B1))),
        s,
    );
    h = _mm_mullo_epi32(_mm_xor_si128(h, _mm_srai_epi32(h, 15)), _mm_set1_epi32(0x2C1B_3C6D));
    h = _mm_mullo_epi32(_mm_xor_si128(h, _mm_srai_epi32(h, 12)), _mm_set1_epi32(0x297A_2D39));
    _mm_xor_si128(h, _mm_srai_epi32(h, 15))
}

#[inline]
#[target_feature(enable = "sse4.1")]
unsafe fn lattice4(h: __m128i) -> __m128 {
    _mm_div_ps(_mm_cvtepi32_ps(_mm_and_si128(h, _mm_set1_epi32(0xFFFF))), _mm_set1_ps(65535.0))
}

#[inline]
#[target_feature(enable = "sse4.1")]
unsafe fn smooth4(t: __m128) -> __m128 {
    _mm_mul_ps(_mm_mul_ps(t, t), _mm_sub_ps(_mm_set1_ps(3.0), _mm_mul_ps(_mm_set1_ps(2.0), t)))
}

#[inline]
#[target_feature(enable = "sse4.1")]
unsafe fn lerp4(a: __m128, b: __m128, t: __m128) -> __m128 {
    _mm_add_ps(a, _mm_mul_ps(_mm_sub_ps(b, a), t))
}

#[inline]
#[target_feature(enable = "sse4.1")]
unsafe fn noise4(x: __m128i, y: __m128i) -> __m128 {
    let (cx, cy) = (_mm_srai_epi32(x, 4), _mm_srai_epi32(y, 4));
    let (fifteen, sixteen, one) = (_mm_set1_epi32(15), _mm_set1_ps(16.0), _mm_set1_epi32(1));
    let fx = smooth4(_mm_div_ps(_mm_cvtepi32_ps(_mm_and_si128(x, fifteen)), sixteen));
    let fy = smooth4(_mm_div_ps(_mm_cvtepi32_ps(_mm_and_si128(y, fifteen)), sixteen));
    let (cx1, cy1) = (_mm_add_epi32(cx, one), _mm_add_epi32(cy, one));
    let a = lattice4(hash4(cx, cy, SEED));
    let b = lattice4(hash4(cx1, cy, SEED));
    let c = lattice4(hash4(cx, cy1, SEED));
    let d = lattice4(hash4(cx1, cy1, SEED));
    let top = lerp4(a, b, fx);
    let bottom = lerp4(c, d, fx);
    lerp4(top, bottom, fy)
}

#[target_feature(enable = "sse4.1")]
unsafe fn bench_wide_sse41(n: i32, iters: i32) -> f64 {
    let mut sum = 0.0f64;
    let lanes = _mm_setr_epi32(0, 1, 2, 3);
    for k in 0..iters {
        for j in 0..n >> 2 {
            let i = j << 2;
            let x = _mm_add_epi32(_mm_set1_epi32(i & 1023), lanes);
            let y = _mm_set1_epi32((i >> 10) + k);
            let mut v = [0.0f32; 4];
            _mm_storeu_ps(v.as_mut_ptr(), noise4(x, y));
            for e in v {
                sum += e as f64;
            }
        }
    }
    sum
}

/// `n * iters` samples, four at a time (`n` a multiple of 4). Needs SSE4.1 (`_mm_mullo_epi32`).
#[inline(never)]
pub fn bench_wide(n: i32, iters: i32) -> f64 {
    assert!(is_x86_feature_detected!("sse4.1"), "noise_wide needs SSE4.1");
    // SAFETY: SSE4.1 is present (checked above).
    unsafe { bench_wide_sse41(n, iters) }
}
