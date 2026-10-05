// SPDX-License-Identifier: Apache-2.0
//! 16.16 fixed point, `GLfixed`: a signed 32-bit number with sixteen
//! bits of fraction, and the rule for every narrowing.
//!
//! A product of two is formed in 64 bits, and a sum of products is
//! summed there before it is narrowed, so a dot product rounds once.
//! Narrowing rounds to the nearest, a half upwards, and saturates
//! rather than wraps, so an overflow distorts a picture rather than
//! throwing a vertex to the other side of the screen
//! (`docs/gles.md`, section 4).

/// A `GLfixed`: sixteen bits of fraction in thirty-two.
pub type Fx = i32;

/// One, in 16.16.
pub const ONE: Fx = 1 << 16;

/// A 64-bit value narrowed to thirty-two bits, saturated.
pub fn sat(v: i64) -> i32 {
    v.clamp(i32::MIN as i64, i32::MAX as i64) as i32
}

/// A sum of products of 16.16 numbers, which has thirty-two bits of
/// fraction, rounded once to 16.16 and saturated.
pub fn narrow(acc: i64) -> Fx {
    sat((acc + (1 << 15)) >> 16)
}

/// The product of two 16.16 numbers.
pub fn mul(a: Fx, b: Fx) -> Fx {
    narrow(a as i64 * b as i64)
}

/// `n / d` rounded to the nearest, a half upwards, for any signs, and
/// `d` not zero.
pub fn div_round(n: i64, d: i64) -> i64 {
    let (n, d) = if d < 0 { (-n, -d) } else { (n, d) };
    (2 * n + d).div_euclid(2 * d)
}

/// The quotient of two 16.16 numbers, or the largest of the quotient's
/// sign when `b` is zero.
pub fn div(a: Fx, b: Fx) -> Fx {
    if b == 0 {
        return if a < 0 { i32::MIN } else { i32::MAX };
    }
    sat(div_round((a as i64) << 16, b as i64))
}

/// The square root of a 64-bit number, rounded down.
pub fn isqrt(v: u64) -> u64 {
    if v < 2 {
        return v;
    }
    // Newton's method from above, which only falls until it settles.
    let mut x = 1u64 << ((64 - v.leading_zeros()).div_ceil(2));
    loop {
        let y = (x + v / x) / 2;
        if y >= x {
            return x;
        }
        x = y;
    }
}

/// Thirty bits of fraction, for the sine and cosine.
const Q: u32 = 30;

/// The product of two numbers with thirty bits of fraction.
fn mulq(a: i64, b: i64) -> i64 {
    (a * b + (1 << (Q - 1))) >> Q
}

/// The sine and cosine of `x` radians, `x` with thirty bits of
/// fraction and between nought and a quarter of pi, by their series to
/// the eleventh power, which is within a billionth there.
fn series(x: i64) -> (i64, i64) {
    let x2 = mulq(x, x);
    let (mut s, mut c) = (0i64, 0i64);
    let (mut ts, mut tc) = (x, 1i64 << Q);
    for k in 0..6i64 {
        s += ts;
        c += tc;
        ts = -mulq(ts, x2) / ((2 * k + 2) * (2 * k + 3));
        tc = -mulq(tc, x2) / ((2 * k + 1) * (2 * k + 2));
    }
    (s, c)
}

/// Pi over 180 with thirty bits of fraction: a degree in radians.
const DEGREE: i64 = 18_740_330;

/// The sine and cosine of an angle of `deg` degrees in 16.16, each in
/// 16.16. The angle is taken to the first eighth of a turn by the
/// symmetries of the two functions, where the series is short.
pub fn sin_cos(deg: Fx) -> (Fx, Fx) {
    let turn = 360i64 << 16;
    let quarter = 90i64 << 16;
    let d = (deg as i64).rem_euclid(turn);
    let (q, r) = (d / quarter, d % quarter);
    // Within the quarter, past its half the two swap.
    let (r, swap) = if r > quarter / 2 {
        (quarter - r, true)
    } else {
        (r, false)
    };
    let (s, c) = series((r * DEGREE) >> 16);
    let (s, c) = if swap { (c, s) } else { (s, c) };
    let (s, c) = match q {
        0 => (s, c),
        1 => (c, -s),
        2 => (-s, -c),
        _ => (-c, s),
    };
    let to16 = |v: i64| ((v + (1 << 13)) >> 14) as Fx;
    (to16(s), to16(c))
}

/// The square root of a 64-bit number, rounded down, at compile time.
const fn isqrt_const(v: u64) -> u64 {
    if v < 2 {
        return v;
    }
    let mut x = v;
    let mut y = x.div_ceil(2);
    while y < x {
        x = y;
        y = (x + v / x) / 2;
    }
    x
}

/// `2^(2^-k)` for `k` from 1 to 16, with thirty bits of fraction: each
/// the square root of the one before, from 2.
const ROOTS: [u64; 16] = {
    let mut r = [0u64; 16];
    let mut c = 2u64 << Q;
    let mut k = 0;
    while k < 16 {
        c = isqrt_const(c << Q);
        r[k] = c;
        k += 1;
    }
    r
};

/// The base 2 logarithm of a positive 16.16 number, in 16.16: the
/// exponent of its leading bit, then the fraction a bit at a time by
/// squaring the mantissa, each square at or past two a one.
pub fn log2(x: Fx) -> i64 {
    debug_assert!(x > 0);
    let top = 31 - x.leading_zeros() as i64;
    // The mantissa, in [1, 2) with thirty bits of fraction.
    let mut m = ((x as u64) << Q) >> top;
    let mut r = (top - 16) << 16;
    for bit in (0..16).rev() {
        m = (m * m) >> Q;
        if m >= 2 << Q {
            m >>= 1;
            r += 1 << bit;
        }
    }
    r
}

/// Two to the power of `y`, a 16.16 number, in 16.16, saturated: the
/// integer part a shift, and the fraction a product of the [`ROOTS`]
/// its bits name.
pub fn exp2(y: i64) -> Fx {
    let n = y >> 16;
    let f = y & 0xffff;
    let mut r = 1u64 << Q;
    for (k, root) in ROOTS.iter().enumerate() {
        if f & (1 << (15 - k)) != 0 {
            r = (r * root + (1 << (Q - 1))) >> Q;
        }
    }
    // r is in [1, 2) with thirty bits of fraction; the result has
    // sixteen, so it is r shifted by n less fourteen.
    let shift = n - (Q as i64 - 16);
    let v = if shift >= 0 {
        if shift > 32 {
            i64::MAX
        } else {
            (r as i64) << shift
        }
    } else if shift < -62 {
        0
    } else {
        ((r >> (-shift - 1)) as i64 + 1) >> 1
    };
    sat(v)
}

/// `x` to the power `y`, both 16.16, `x` from nought to one and `y`
/// from nought to 128, as the shininess and the spot exponent are:
/// `2^(y log2 x)`. Nought to the power nought is one, as the
/// specification has it.
pub fn pow(x: Fx, y: Fx) -> Fx {
    if y == 0 {
        return ONE;
    }
    if x <= 0 {
        return 0;
    }
    exp2((log2(x) * y as i64) >> 16)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn f(v: Fx) -> f64 {
        v as f64 / 65536.0
    }

    #[test]
    fn a_product_rounds_once_and_saturates() {
        assert_eq!(mul(ONE + ONE / 2, ONE * 2), 3 * ONE);
        assert_eq!(mul(1, ONE / 2), 1, "a half rounds up");
        assert_eq!(mul(-1, ONE / 2), 0, "and so does a negative half");
        assert_eq!(mul(i32::MAX, 4 * ONE), i32::MAX, "saturated");
        assert_eq!(mul(i32::MIN, 4 * ONE), i32::MIN, "both ways");
    }

    #[test]
    fn a_quotient_rounds_to_the_nearest() {
        assert_eq!(div(ONE, 3 * ONE), 21845);
        assert_eq!(div(2 * ONE, 3 * ONE), 43691);
        assert_eq!(div(-ONE, 3 * ONE), -21845);
        assert_eq!(div(ONE, 0), i32::MAX);
        assert_eq!(div_round(-3, 2), -1, "a half upwards");
        assert_eq!(div_round(3, -2), -1, "whatever the signs");
    }

    #[test]
    fn a_square_root_is_rounded_down() {
        for v in [0u64, 1, 2, 3, 4, 15, 16, 17, 1 << 40, (1 << 62) + 12345] {
            let r = isqrt(v);
            assert!(r * r <= v && (r + 1) * (r + 1) > v, "{v}: {r}");
        }
        assert_eq!(isqrt(u64::MAX), 0xffff_ffff);
    }

    /// Every tenth of a degree, a little off it, within a unit
    /// of the last place of the true sine and cosine.
    #[test]
    fn sine_and_cosine_are_within_a_unit_of_the_last_place() {
        for tenth in -3700..3700 {
            let deg = tenth * ONE / 10 + 123;
            let (s, c) = sin_cos(deg);
            let r = f(deg).to_radians();
            let err = |got: Fx, want: f64| (f(got) - want).abs() * 65536.0;
            assert!(err(s, r.sin()) <= 1.0, "sin {}: {s}", f(deg));
            assert!(err(c, r.cos()) <= 1.0, "cos {}: {c}", f(deg));
        }
    }
    /// Logarithms and powers of two within a few units of the last
    /// place, and powers of numbers in nought to one, the shininess's
    /// range, within a few thousandths of floating point.
    #[test]
    fn logarithms_and_powers_agree_with_floating_point() {
        for x in [1, 7, 300, 32768, 65536, 70000, 1 << 20, i32::MAX] {
            let want = f(x).log2() * 65536.0;
            assert!((log2(x) as f64 - want).abs() <= 2.0, "log2 {}", f(x));
        }
        for y in [-20 * ONE, -ONE / 3, 0, ONE / 2, 5 * ONE + 12345, 14 * ONE] {
            let want = 2f64.powf(f(y)) * 65536.0;
            let got = exp2(y as i64) as f64;
            assert!(
                (got - want).abs() <= 2.0 + want * 2e-5,
                "exp2 {}: {got}",
                f(y)
            );
        }
        for x in [0.0, 0.01, 0.2, 0.5, 0.9, 0.99, 1.0] {
            for y in [0.0, 0.5, 1.0, 3.0, 10.0, 50.5, 128.0] {
                let got = f(pow((x * 65536.0) as Fx, (y * 65536.0) as Fx));
                let want = (x * 65536.0f64).floor() / 65536.0;
                let want = if y == 0.0 { 1.0 } else { want.powf(y) };
                assert!(
                    (got - want).abs() <= 2e-3,
                    "{x}^{y}: {got} for {want}"
                );
            }
        }
        assert_eq!(exp2(40 * ONE as i64), i32::MAX, "saturated");
        assert_eq!(exp2(-80 * ONE as i64), 0);
    }
}
