// SPDX-License-Identifier: Apache-2.0
//! 4 by 4 matrices in 16.16, stored as GL stores them, a column after a
//! column, and the matrices GL's calls make.

use crate::fixed::{div, isqrt, mul, narrow, sin_cos, Fx, ONE};

/// A matrix, column-major: element `(row, col)` is `m[col * 4 + row]`.
pub type Mat = [Fx; 16];

/// The identity.
pub const IDENTITY: Mat =
    [ONE, 0, 0, 0, 0, ONE, 0, 0, 0, 0, ONE, 0, 0, 0, 0, ONE];

/// The product `a * b`, each element a dot product rounded once.
pub fn mul_mat(a: &Mat, b: &Mat) -> Mat {
    let mut r = [0; 16];
    for col in 0..4 {
        for row in 0..4 {
            let mut acc = 0i64;
            for k in 0..4 {
                acc += a[k * 4 + row] as i64 * b[col * 4 + k] as i64;
            }
            r[col * 4 + row] = narrow(acc);
        }
    }
    r
}

/// `m` times the column vector `v`, each component rounded once.
pub fn mul_vec(m: &Mat, v: &[Fx; 4]) -> [Fx; 4] {
    let mut r = [0; 4];
    for (row, out) in r.iter_mut().enumerate() {
        let mut acc = 0i64;
        for (k, x) in v.iter().enumerate() {
            acc += m[k * 4 + row] as i64 * *x as i64;
        }
        *out = narrow(acc);
    }
    r
}

/// A matrix from its rows, as the specification writes them.
fn rows(r: [[Fx; 4]; 4]) -> Mat {
    let mut m = [0; 16];
    for (row, line) in r.iter().enumerate() {
        for (col, v) in line.iter().enumerate() {
            m[col * 4 + row] = *v;
        }
    }
    m
}

/// `glTranslatex`'s matrix.
pub fn translate(x: Fx, y: Fx, z: Fx) -> Mat {
    rows([
        [ONE, 0, 0, x],
        [0, ONE, 0, y],
        [0, 0, ONE, z],
        [0, 0, 0, ONE],
    ])
}

/// `glScalex`'s matrix.
pub fn scale(x: Fx, y: Fx, z: Fx) -> Mat {
    rows([[x, 0, 0, 0], [0, y, 0, 0], [0, 0, z, 0], [0, 0, 0, ONE]])
}

/// `glRotatex`'s matrix: `deg` degrees about the axis `(x, y, z)`,
/// which need not be of unit length. An axis of no length is no turn.
pub fn rotate(deg: Fx, x: Fx, y: Fx, z: Fx) -> Mat {
    let sq = |v: Fx| (v as i64 * v as i64) as u64;
    let len = isqrt(sq(x) + sq(y) + sq(z)) as i64;
    if len == 0 {
        return IDENTITY;
    }
    let unit = |v: Fx| {
        crate::fixed::sat(crate::fixed::div_round((v as i64) << 16, len))
    };
    let (x, y, z) = (unit(x), unit(y), unit(z));
    let (s, c) = sin_cos(deg);
    let k = ONE - c;
    let (xs, ys, zs) = (mul(x, s), mul(y, s), mul(z, s));
    let (xk, yk, zk) = (mul(x, k), mul(y, k), mul(z, k));
    rows([
        [mul(x, xk) + c, mul(x, yk) - zs, mul(x, zk) + ys, 0],
        [mul(y, xk) + zs, mul(y, yk) + c, mul(y, zk) - xs, 0],
        [mul(z, xk) - ys, mul(z, yk) + xs, mul(z, zk) + c, 0],
        [0, 0, 0, ONE],
    ])
}

/// `glFrustumx`'s matrix, or `None` for the values the specification
/// refuses: a near or far plane not in front of the eye, or a box of
/// no width, height or depth.
pub fn frustum(l: Fx, r: Fx, b: Fx, t: Fx, n: Fx, f: Fx) -> Option<Mat> {
    if n <= 0 || f <= 0 || l == r || b == t || n == f {
        return None;
    }
    let (rl, tb, fne) = (r - l, t - b, f - n);
    let two_n = 2 * n as i64;
    let q = |a: i64, d: Fx| {
        crate::fixed::sat(crate::fixed::div_round(a << 16, d as i64))
    };
    Some(rows([
        [q(two_n, rl), 0, div(r + l, rl), 0],
        [0, q(two_n, tb), div(t + b, tb), 0],
        [0, 0, -div(f + n, fne), -q(2 * mul(f, n) as i64, fne)],
        [0, 0, -ONE, 0],
    ]))
}

/// `glOrthox`'s matrix, or `None` for a box of no width, height or
/// depth.
pub fn ortho(l: Fx, r: Fx, b: Fx, t: Fx, n: Fx, f: Fx) -> Option<Mat> {
    if l == r || b == t || n == f {
        return None;
    }
    let (rl, tb, fne) = (r - l, t - b, f - n);
    Some(rows([
        [div(2 * ONE, rl), 0, 0, -div(r + l, rl)],
        [0, div(2 * ONE, tb), 0, -div(t + b, tb)],
        [0, 0, -div(2 * ONE, fne), -div(f + n, fne)],
        [0, 0, 0, ONE],
    ]))
}

/// A plane in object coordinates carried into eye coordinates by the
/// modelview `m`: the plane times the inverse of `m`, as `glClipPlane`
/// says. Worked out in 128 bits from the adjugate, since it happens
/// once a call and not once a vertex. `None` when `m` has no inverse.
/// The 128 bits hold every product for a modelview whose elements are
/// under 256 in magnitude; past that a product can overflow, which is
/// undefined in the specification.
pub fn plane_to_eye(m: &Mat, p: &[Fx; 4]) -> Option<[Fx; 4]> {
    let a = |row: usize, col: usize| m[col * 4 + row] as i128;
    // The cofactor of (row, col): the determinant of the 3 by 3 left
    // when the row and the column are struck out, signed.
    let cof = |row: usize, col: usize| {
        let r: [usize; 3] =
            core::array::from_fn(|i| if i < row { i } else { i + 1 });
        let c: [usize; 3] =
            core::array::from_fn(|i| if i < col { i } else { i + 1 });
        let e = |i: usize, j: usize| a(r[i], c[j]);
        let d = e(0, 0) * (e(1, 1) * e(2, 2) - e(1, 2) * e(2, 1))
            - e(0, 1) * (e(1, 0) * e(2, 2) - e(1, 2) * e(2, 0))
            + e(0, 2) * (e(1, 0) * e(2, 1) - e(1, 1) * e(2, 0));
        if (row + col).is_multiple_of(2) {
            d
        } else {
            -d
        }
    };
    let det: i128 = (0..4).map(|col| a(0, col) * cof(0, col)).sum();
    if det == 0 {
        return None;
    }
    // The inverse's (i, j) is cof(j, i) / det. The plane's eye form is
    // the row vector p times the inverse: out[j] = sum_i p[i] inv(i, j)
    // = sum_i p[i] cof(j, i) / det. The cofactors carry 48 bits of
    // fraction, the determinant 64 and the plane 16, so the sum over
    // the determinant has none, and is shifted up 16 for 16.16.
    let mut out = [0; 4];
    for (j, o) in out.iter_mut().enumerate() {
        let num: i128 =
            (0..4).map(|i| p[i] as i128 * cof(j, i)).sum::<i128>() << 16;
        let (n, d) = if det < 0 { (-num, -det) } else { (num, det) };
        let q = (2 * n + d).div_euclid(2 * d);
        *o = q.clamp(i32::MIN as i128, i32::MAX as i128) as i32;
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn f(v: Fx) -> f64 {
        v as f64 / 65536.0
    }

    /// Element by element, within `tol` units of the last place.
    fn close(got: &Mat, want: [[f64; 4]; 4], tol: f64, what: &str) {
        for row in 0..4 {
            for col in 0..4 {
                let g = f(got[col * 4 + row]);
                let e = (g - want[row][col]).abs() * 65536.0;
                assert!(
                    e <= tol,
                    "{what} ({row},{col}): {g} for {}",
                    want[row][col]
                );
            }
        }
    }

    #[test]
    fn rotation_agrees_with_floating_point() {
        for (deg, ax) in [
            (30.0, (0.0, 0.0, 1.0)),
            (-75.5, (1.0, 2.0, 3.0)),
            (200.25, (0.5, -1.0, 0.25)),
            (90.0, (0.0, 1.0, 0.0)),
        ] {
            let fx = |v: f64| (v * 65536.0).round() as Fx;
            let m = rotate(fx(deg), fx(ax.0), fx(ax.1), fx(ax.2));
            let n = (ax.0 * ax.0 + ax.1 * ax.1 + ax.2 * ax.2).sqrt();
            let (x, y, z) = (ax.0 / n, ax.1 / n, ax.2 / n);
            let (s, c) = deg.to_radians().sin_cos();
            let k = 1.0 - c;
            let want = [
                [x * x * k + c, x * y * k - z * s, x * z * k + y * s, 0.0],
                [y * x * k + z * s, y * y * k + c, y * z * k - x * s, 0.0],
                [z * x * k - y * s, z * y * k + x * s, z * z * k + c, 0.0],
                [0.0, 0.0, 0.0, 1.0],
            ];
            close(&m, want, 4.0, &format!("rotate {deg}"));
        }
    }

    #[test]
    fn frustum_and_ortho_agree_with_floating_point() {
        let fx = |v: f64| (v * 65536.0).round() as Fx;
        let (l, r, b, t, n, fa) = (-0.6, 0.5, -0.4, 0.45, 0.75, 20.0);
        let m = frustum(fx(l), fx(r), fx(b), fx(t), fx(n), fx(fa)).unwrap();
        let want = [
            [2.0 * n / (r - l), 0.0, (r + l) / (r - l), 0.0],
            [0.0, 2.0 * n / (t - b), (t + b) / (t - b), 0.0],
            [0.0, 0.0, -(fa + n) / (fa - n), -2.0 * fa * n / (fa - n)],
            [0.0, 0.0, -1.0, 0.0],
        ];
        close(&m, want, 2.0, "frustum");
        let o =
            ortho(fx(0.0), fx(640.0), fx(0.0), fx(480.0), fx(-1.0), fx(1.0))
                .unwrap();
        let want = [
            [2.0 / 640.0, 0.0, 0.0, -1.0],
            [0.0, 2.0 / 480.0, 0.0, -1.0],
            [0.0, 0.0, -1.0, 0.0],
            [0.0, 0.0, 0.0, 1.0],
        ];
        close(&o, want, 1.0, "ortho");
        assert_eq!(frustum(0, ONE, 0, ONE, 0, ONE), None, "near at the eye");
        assert_eq!(ortho(0, 0, 0, ONE, 0, ONE), None, "no width");
    }

    /// A plane carried by a modelview holds the same points it held:
    /// a point on the plane in object space is on it in eye space.
    #[test]
    fn a_plane_follows_the_modelview() {
        let m = mul_mat(
            &translate(3 * ONE, -ONE, 2 * ONE),
            &mul_mat(
                &rotate(40 * ONE, ONE, ONE, 0),
                &scale(2 * ONE, ONE, ONE / 2),
            ),
        );
        // The plane x + 2y - z - 1 = 0, and three points on it.
        let p = [ONE, 2 * ONE, -ONE, -ONE];
        let e = plane_to_eye(&m, &p).unwrap();
        for v in [
            [ONE, 0, 0, ONE],
            [0, ONE, ONE, ONE],
            [3 * ONE, 0, 2 * ONE, ONE],
        ] {
            let ev = mul_vec(&m, &v);
            let d: i64 = (0..4).map(|i| e[i] as i64 * ev[i] as i64).sum();
            assert!((d >> 16).abs() <= 8, "a point on the plane is {d} off it");
        }
        assert_eq!(plane_to_eye(&scale(0, ONE, ONE), &p), None);
    }

    #[test]
    fn products_compose_as_the_specification_says() {
        let t = translate(ONE, 2 * ONE, 3 * ONE);
        let s = scale(2 * ONE, 2 * ONE, 2 * ONE);
        let v = [ONE, ONE, ONE, ONE];
        // Scale first, then move: (2, 2, 2) + (1, 2, 3).
        assert_eq!(
            mul_vec(&mul_mat(&t, &s), &v),
            [3 * ONE, 4 * ONE, 5 * ONE, ONE]
        );
        assert_eq!(mul_mat(&IDENTITY, &t), t);
    }
}
