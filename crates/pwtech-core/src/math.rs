//! Logarithms exactly as computed by the original executable.
//!
//! - `Math.Log2` of the Delphi RTL is `Ln(x) * (1/ln 2)` with the RTL's own
//!   `Ln` (so `Log2(8192) = 12.999999999999998`, not 13);
//! - `log()` of the C library (Dinkumware) is used in `CalcPermSetEntropy`.
//!
//! Both were reconstructed operation by operation from the disassembly of
//! PwTech.exe 3.6.1 (see `tools/refharness/shim/vcl/vclshim.cpp` and
//! `tools/refharness/shim/dinkum_compat.cpp`). Rust does not contract
//! floating-point operations, so each step rounds like the original SSE2 code.

#[inline]
fn bits(u: u64) -> f64 {
    f64::from_bits(u)
}

/// `log(1+f)` kernel of the Delphi RTL for f in [sqrt(2)/2-1, sqrt(2)-1].
fn delphi_ln_kernel(f: f64) -> f64 {
    const P: [u64; 12] = [
        0x3fb0c835bd538f8c,
        0x3fa988ab1ef94f29,
        0x3fae3c098f53b5ee,
        0x3fb11053fca919d4,
        0x3fb3b140d1e4050f,
        0x3fb745d159460dd7,
        0x3fbc71c71cbb0290,
        0x3fc249249248f262,
        0x3c602e384367005f,
        0x3fc99999999999b4,
        0x3c750aaac32dac90,
        0x3fd5555555555555,
    ];
    let p = |i: usize| bits(P[i]);
    let ff = std::hint::black_box(f);
    let s = ff / (ff + 2.0);
    let half_f = ff * 0.5;
    let hfsq = ff * half_f;
    let x3 = hfsq * half_f;
    let z = s * s;
    let w = z * z;
    let mut t1 = p(0) * w + p(2);
    let mut t2 = p(1) * w + p(3);
    t1 = t1 * w + p(4);
    t2 = t2 * w + p(5);
    t1 = t1 * w + p(6);
    t2 = t2 * w + p(7);
    t1 = t1 * w + p(8);
    t1 += p(9);
    t2 = t2 * w + p(10);
    t1 *= z;
    let mut r = t2 + t1;
    r += p(11);
    r *= 2.0;
    r *= z;
    r *= s;
    let sx3 = s * x3;
    r += x3;
    r -= hfsq;
    r -= sx3;
    r += ff;
    r
}

/// `System.Ln` of the Delphi Win64 RTL.
pub fn delphi_ln(x: f64) -> f64 {
    let b = x.to_bits();
    if (b >> 63) != 0 || b == 0 || b >= 0x7ff0_0000_0000_0000 {
        return x.ln();
    }
    let mut mant = b & 0x000f_ffff_ffff_ffff;
    let exponent = ((b >> 52) & 0x7ff) as i32;
    let mut k: i32;
    if exponent == 0 {
        k = -1022;
        while mant & 0x0010_0000_0000_0000 == 0 {
            mant <<= 1;
            k -= 1;
        }
        mant &= 0x000f_ffff_ffff_ffff;
    } else {
        k = exponent - 1023;
    }
    let m = if mant <= 0x6a09e667f3bcc {
        bits(0x3ff0_0000_0000_0000 + mant)
    } else {
        k += 1;
        bits(0x3fe0_0000_0000_0000 + mant)
    };
    let mut r = delphi_ln_kernel(m - 1.0);
    let dk = k as f64;
    r += dk * bits(0x3e37d1cf79abc9e4);
    r += dk * bits(0x3fe62e42fc000000);
    r
}

/// `Math.Log2` of the Delphi RTL: `Ln(x) * 1/ln(2)`.
pub fn delphi_log2(x: f64) -> f64 {
    delphi_ln(x) * 1.4426950408889634
}

/// `log()` of the Dinkumware C library (`_Log(x, 0)`).
pub fn dinkum_log(x: f64) -> f64 {
    if x.is_nan() {
        return x;
    }
    if x == 0.0 {
        return f64::NEG_INFINITY;
    }
    if x < 0.0 {
        return f64::NAN;
    }
    if x == f64::INFINITY {
        return x;
    }
    // frexp: x = m * 2^e with m in [0.5, 1)
    let (mut m, mut e) = frexp(x);
    if m < bits(0x3fe6a09e667f3bcd) {
        m = m + m;
        e -= 1;
    }
    const C: [u64; 7] = [
        0x3fc2f8d8bf29def7,
        0x3fc3991accfeb3f4,
        0x3fc7466fd968cb62,
        0x3fcc71c4da12c9c6,
        0x3fd24924948ae94a,
        0x3fd9999999975ff7,
        0x3fe55555555555bc,
    ];
    let c = |i: usize| bits(C[i]);
    let mm = std::hint::black_box(m);
    let f = -1.0 + mm;
    let d = mm + 1.0;
    let z = f / d;
    let w = z * z;
    let mut p = c(0) * w;
    p = (p + c(1)) * w;
    p = (p + c(2)) * w;
    p = (p + c(3)) * w;
    p = (p + c(4)) * w;
    p = (p + c(5)) * w;
    p = (p + c(6)) * w;
    let mut t = f - p;
    t *= z;
    let k = e as f64;
    let lo = bits(0xbe7f7d1cf79abc9e) * k;
    t += lo;
    let r1 = f - t;
    let hi = k * bits(0x3fe62e42c0000000);
    hi + r1
}

/// `frexp()` for finite positive values (exact).
fn frexp(x: f64) -> (f64, i32) {
    let b = x.to_bits();
    let exp = ((b >> 52) & 0x7ff) as i32;
    if exp == 0 {
        // subnormal: scale up first
        let (m, e) = frexp(x * bits(0x4350_0000_0000_0000)); // 2^54
        return (m, e - 54);
    }
    let m = bits((b & 0x800f_ffff_ffff_ffff) | (0x3fe << 52));
    (m, exp - 1022)
}

/// `log(2.0)` folded at compile time by the original compiler
/// (`0x3FE62E42FEFA39EF`).
pub const LOG2_CONST: f64 = std::f64::consts::LN_2;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_values_from_original() {
        // Values reported by the reference harness (bits of the original).
        assert_eq!(delphi_log2(8192.0), 12.999999999999998);
        assert_eq!(delphi_log2(64.0), 5.999999999999999);
        assert!((delphi_log2(62.0) - 62f64.log2()).abs() < 1e-14);
        assert!((dinkum_log(10.0) - 10f64.ln()).abs() < 1e-15);
        assert_eq!(LOG2_CONST.to_bits(), 0x3FE62E42FEFA39EF);
        let (m, e) = frexp(12.0);
        assert_eq!((m, e), (0.75, 4));
    }
}
