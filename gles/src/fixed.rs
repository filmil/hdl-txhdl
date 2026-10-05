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
}
