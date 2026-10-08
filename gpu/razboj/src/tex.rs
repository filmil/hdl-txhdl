// SPDX-License-Identifier: Apache-2.0
//! Texturing as the rasteriser does it (issue 997), written with integers
//! so that it states the arithmetic the hardware is held to, bit for bit,
//! and is held in its turn to GL's formulas in floating point.
//!
//! A pixel's texture coordinates come from three planes the walk steps,
//! `u q` and `v q` with 32 bits of fraction and `q` with 48 (see
//! `razboj_tile::tex`): `u = (u q) / q`, through a reciprocal of `q` from
//! a line in each of 32 segments and one Newton step. The level of detail is
//! `log2` of the largest numerator of `u`'s and `v`'s derivatives, less
//! twice `log2 q`, both through a table of 256 for the fraction. Then
//! GL's filters, wrap modes and environments.
use crate::model::div255;
use crate::raster::{log_rise, log_start, seed_fall, seed_start};
use razboj_tile::tex::{
    level_base, side, texel_offset, Desc, ADD, ALPHA, BLEND, DECAL, LINEAR,
    LINEAR_MIPMAP_NEAREST, LUMINANCE, LUMINANCE_ALPHA, MODULATE, NEAREST,
    NEAREST_MIPMAP_LINEAR, NEAREST_MIPMAP_NEAREST, REPLACE, RGB,
};
use txhdl::types::U;

/// The line a segment of the reciprocal's first guess follows, for the
/// segment `k` of 32 between one and two: its value at the segment's
/// start, `a`, and its fall over the segment's 1024 steps, `b` in 1024ths
/// of a step, both with 16 bits of fraction. It is the chord of `2^17 /
/// m` over the segment, lowered by half the most it lies above the curve,
/// so that it errs as far each way.
pub fn seed_line(k: u32) -> (u32, u32) {
    let f =
        |t: f64| (1u64 << 17) as f64 / (1.0 + (k as f64 + t / 1024.0) / 32.0);
    let s = (f(0.0) - f(1024.0)) / 1024.0;
    let over = (0..=1024)
        .map(|t| f(0.0) - s * t as f64 - f(t as f64))
        .fold(0f64, f64::max);
    (
        (f(0.0) - over / 2.0).round() as u32,
        (s * 1024.0).round() as u32,
    )
}

/// The reciprocal's first guess for the mantissa `x`, in `[2^31, 2^32)`,
/// with 16 bits of fraction: the line of the segment its five bits below
/// the leading one name, at the ten bits below those. The lines are the
/// rasteriser's own, which a test holds to [`seed_line`].
pub fn seed(x: u64) -> u64 {
    let k = U::<5>::from(((x >> 26) & 31) as u32);
    let (a, b) = (seed_start(k).raw() as u64, seed_fall(k).raw() as u64);
    a - ((b * ((x >> 16) & 1023)) >> 10)
}

/// The line a segment of `log2`'s fraction follows, for the segment `k`
/// of 32 between one and two: its value at the segment's start, `a`, and
/// its rise over the segment's 1024 steps, `b`, both with 16 bits of
/// fraction. It is the chord of `65536 log2 m` over the segment, raised
/// by half the most it lies below the curve, so that it errs as far each
/// way.
pub fn log_line(k: u32) -> (u32, u32) {
    let f = |t: f64| 65536.0 * (1.0 + (k as f64 + t / 1024.0) / 32.0).log2();
    let s = (f(1024.0) - f(0.0)) / 1024.0;
    let under = (0..=1024)
        .map(|t| f(t as f64) - f(0.0) - s * t as f64)
        .fold(0f64, f64::max);
    (
        (f(0.0) + under / 2.0).round() as u32,
        (s * 1024.0).round() as u32,
    )
}

/// The fraction of `log2` for the mantissa `x`, its leading one at bit
/// 31, in 8 bits: the line of the segment its five bits below the
/// leading one name, at the ten bits below those, rounded. The lines are
/// the rasteriser's own, which a test holds to [`log_line`].
pub fn log_frac(x: u32) -> i32 {
    let k = U::<5>::from((x >> 26) & 31);
    let (a, b) = (log_start(k).raw() as u32, log_rise(k).raw() as u32);
    let t = (x >> 16) & 1023;
    ((a + ((b * t) >> 10) + 128) >> 8) as i32
}

/// `q`, with 48 bits of fraction and at least one, normalised: its
/// leading zeros, and its top 32 bits from its leading one.
fn normal(q: u64) -> (u32, u64) {
    let q = q.max(1);
    let n = q.leading_zeros();
    (n, (q << n) >> 32)
}

/// The reciprocal of the mantissa `x`, in `[2^31, 2^32)`, with 24 bits of
/// fraction: the first guess, then one Newton step,
/// `r (2 - x r)`.
pub fn recip(x: u64) -> u64 {
    let r0 = seed(x);
    let p = x * r0;
    let e = (1u64 << 49) - p;
    ((r0 as u128 * e as u128) >> 40) as u64
}

/// A pixel's texel coordinates, in texels of the base level with 8 bits of
/// fraction, from its `u q`, `v q` and `q`: each times the reciprocal of
/// `q`'s mantissa, shifted by `q`'s leading zeros, and kept within 2^30.
pub fn texel_uv(uq: u64, vq: u64, q: u64) -> (i32, i32) {
    let (n, x) = normal(q);
    let r = recip(x) as i128;
    let shift = 64 - n;
    let at = |p: u64| {
        let v = (p as i64 as i128 * r) >> shift;
        v.clamp(-(1 << 30), 1 << 30) as i32
    };
    (at(uq), at(vq))
}

/// `log2` of `q`'s value, `q` with 48 bits of fraction, in 8.8: its
/// exponent from its leading zeros, and the fraction from its line.
pub fn log2q(q: u64) -> i32 {
    let (n, x) = normal(q);
    (15 - n as i32) * 256 + log_frac(x as u32)
}

/// `log2` of a magnitude of 32 bits, in 8.8, through the same lines.
pub fn log2u32(v: u32) -> i32 {
    let n = v.max(1).leading_zeros();
    (31 - n as i32) * 256 + log_frac(v.max(1) << n)
}

/// The level of detail below every level, for a pixel whose derivatives
/// are all nought: it is magnified.
pub const NO_LOD: i32 = -(1 << 20);

/// A pixel's level of detail, in 8.8: `log2` of the largest of the four
/// numerators `n` of `u`'s and `v`'s derivatives there, shifted right by
/// `k`, less twice `log2 q`, less the 80 bits the units of `u q` and `q`
/// leave (see `razboj_tile::tex`). That is `log2 ρ` with GL's
/// `ρ = max(|∂u/∂x|, |∂u/∂y|, |∂v/∂x|, |∂v/∂y|)`.
pub fn lod(n: [i32; 4], k: u32, q: u64) -> i32 {
    let m = n.iter().map(|v| v.unsigned_abs()).max().unwrap_or(0);
    if m == 0 {
        return NO_LOD;
    }
    log2u32(m) + (k as i32 - 80) * 256 - 2 * log2q(q)
}

/// The numerators of an entry's derivatives at the pixel `i` right and
/// `j` down of its box's first, stepped as the walk steps them.
pub fn numerators(i: &crate::op::Insn, x: i32, y: i32) -> [i32; 4] {
    let r = |u: txhdl::types::U<32>| u.raw() as u32;
    let at =
        |v, d, by: i32| r(v).wrapping_add(r(d).wrapping_mul(by as u32)) as i32;
    [
        at(i.nux, i.nuxd, y),
        at(i.nvx, i.nvxd, y),
        at(i.nuy, i.nuyd, x),
        at(i.nvy, i.nvyd, x),
    ]
}

/// A texel coordinate wrapped onto a side of `size` texels: repeated, or
/// clamped to the edge.
fn wrap(i: i32, size: u32, clamp: bool) -> u32 {
    if clamp {
        i.clamp(0, size as i32 - 1) as u32
    } else {
        (i as u32) & (size - 1)
    }
}

/// The texel `(i, j)` of a level, wrapped, read from the texture's memory.
fn texel(
    d: &Desc,
    level: u32,
    i: i32,
    j: i32,
    mem: &dyn Fn(u32) -> u32,
) -> u32 {
    let (w, h) = (side(d.log_w, level), side(d.log_h, level));
    let (i, j) = (wrap(i, w, d.clamp_s), wrap(j, h, d.clamp_t));
    mem(level_base(d, level) + texel_offset(d, level, i, j))
}

/// The four channels of `c`, a byte each, alpha highest.
fn bytes(c: u32) -> [u32; 4] {
    [0, 8, 16, 24].map(|at| (c >> at) & 0xff)
}

/// Four channels back into a word.
fn word(b: [u32; 4]) -> u32 {
    b[0] | b[1] << 8 | b[2] << 16 | b[3] << 24
}

/// One level sampled at `(u, v)`, texels with 8 bits of fraction at the
/// base level, with `filter`, nearest or linear: the coordinates halved
/// for each level down, and four texels weighted in 8 bits each for
/// linear, as GL's formula says with its `- 1/2`.
fn sample_level(
    d: &Desc,
    level: u32,
    u: i32,
    v: i32,
    filter: u32,
    mem: &dyn Fn(u32) -> u32,
) -> u32 {
    let (u, v) = (u >> level, v >> level);
    if filter == NEAREST {
        return texel(d, level, u >> 8, v >> 8, mem);
    }
    let (u, v) = (u - 128, v - 128);
    let (i, j, a, b) = (u >> 8, v >> 8, (u & 255) as u32, (v & 255) as u32);
    let t = [(0, 0), (1, 0), (0, 1), (1, 1)]
        .map(|(di, dj)| bytes(texel(d, level, i + di, j + dj, mem)));
    let w = [(256 - a) * (256 - b), a * (256 - b), (256 - a) * b, a * b];
    word(core::array::from_fn(|c| {
        ((0..4).map(|k| w[k] * t[k][c]).sum::<u32>() + 32768) >> 16
    }))
}

/// A texture sampled at `(u, v)` with the level of detail `lod`, as GL's
/// minification and magnification say: the base level magnified, or a
/// level or two chosen by `lod` and filtered, two blended by `lod`'s
/// fraction for the `_MIPMAP_LINEAR` filters.
pub fn sample(
    d: &Desc,
    u: i32,
    v: i32,
    lod: i32,
    mem: &dyn Fn(u32) -> u32,
) -> u32 {
    let c = if d.mag == LINEAR
        && (d.min == NEAREST_MIPMAP_NEAREST || d.min == NEAREST_MIPMAP_LINEAR)
    {
        128
    } else {
        0
    };
    if lod <= c {
        return sample_level(d, 0, u, v, d.mag, mem);
    }
    let top = d.levels.max(1) as i32 - 1;
    let within = |l: i32| l.clamp(0, top) as u32;
    match d.min {
        NEAREST | LINEAR => sample_level(d, 0, u, v, d.min, mem),
        NEAREST_MIPMAP_NEAREST | LINEAR_MIPMAP_NEAREST => {
            let f = if d.min == NEAREST_MIPMAP_NEAREST {
                NEAREST
            } else {
                LINEAR
            };
            // GL's ceil(λ + 1/2) - 1, nought for λ at most a half.
            let l = if lod <= 128 {
                0
            } else {
                ((lod + 128 + 255) >> 8) - 1
            };
            sample_level(d, within(l), u, v, f, mem)
        }
        _ => {
            let f = if d.min == NEAREST_MIPMAP_LINEAR {
                NEAREST
            } else {
                LINEAR
            };
            let l = lod >> 8;
            if l >= top {
                return sample_level(d, within(top), u, v, f, mem);
            }
            let (t1, t2) = (
                bytes(sample_level(d, within(l), u, v, f, mem)),
                bytes(sample_level(d, within(l + 1), u, v, f, mem)),
            );
            let fr = (lod & 255) as u32;
            word(core::array::from_fn(|c| {
                ((256 - fr) * t1[c] + fr * t2[c] + 128) >> 8
            }))
        }
    }
}

/// GL ES 1.1's texture environments, a channel at a time: the fragment's
/// colour `cf`, the texel `ct` read as its class says, and the
/// environment's colour `cc`, each `0xAARRGGBB`. A product is over 255,
/// rounded, as the blend's is; `DECAL` on a class GL leaves undefined
/// passes the fragment through.
pub fn env(mode: u32, class: u32, cf: u32, ct: u32, cc: u32) -> u32 {
    let (f, t, e) = (bytes(cf), bytes(ct), bytes(cc));
    let mul = |a: u32, b: u32| div255(a * b);
    let lerp = |c: usize| div255(f[c] * (255 - t[c]) + e[c] * t[c]);
    let rgb = |g: &dyn Fn(usize) -> u32| [g(0), g(1), g(2)];
    // Each class's colour and alpha, as GL's table has them.
    let (colour, alpha): ([u32; 3], u32) = match (class, mode) {
        (ALPHA, REPLACE) => (rgb(&|c| f[c]), t[3]),
        (ALPHA, DECAL) => (rgb(&|c| f[c]), f[3]),
        (ALPHA, _) => (rgb(&|c| f[c]), mul(f[3], t[3])),
        (LUMINANCE | RGB, REPLACE) => (rgb(&|c| t[c]), f[3]),
        (LUMINANCE | RGB, MODULATE) => (rgb(&|c| mul(f[c], t[c])), f[3]),
        (LUMINANCE, DECAL) => (rgb(&|c| f[c]), f[3]),
        (RGB, DECAL) => (rgb(&|c| t[c]), f[3]),
        (LUMINANCE | RGB, BLEND) => (rgb(&lerp), f[3]),
        (LUMINANCE | RGB, _) => (rgb(&|c| (f[c] + t[c]).min(255)), f[3]),
        (LUMINANCE_ALPHA, DECAL) => (rgb(&|c| f[c]), f[3]),
        (_, REPLACE) => (rgb(&|c| t[c]), t[3]),
        (_, MODULATE) => (rgb(&|c| mul(f[c], t[c])), mul(f[3], t[3])),
        (_, DECAL) => {
            (rgb(&|c| div255(f[c] * (255 - t[3]) + t[c] * t[3])), f[3])
        }
        (_, BLEND) => (rgb(&lerp), mul(f[3], t[3])),
        (_, ADD) | (_, _) => {
            (rgb(&|c| (f[c] + t[c]).min(255)), mul(f[3], t[3]))
        }
    };
    word([colour[0], colour[1], colour[2], alpha])
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The reciprocal, through the first guess and one Newton step, is
    /// within 2^-23 of the true one over every mantissa, at both ends of
    /// each step the guess takes, and the guess within 2^-12.
    #[test]
    fn the_reciprocal_is_close() {
        let (mut worst, mut first) = (0f64, 0f64);
        for at in 0..(1u64 << 15) {
            for end in [0u64, (1 << 16) - 1] {
                let x = (1u64 << 31) + (at << 16) + end;
                let got = recip(x) as f64 / (1u64 << 24) as f64;
                let want = (1u64 << 32) as f64 / x as f64;
                worst = worst.max(((got - want) / want).abs());
                let guess = seed(x) as f64 / (1u64 << 16) as f64;
                first = first.max(((guess - want) / want).abs());
            }
        }
        assert!(first < 1.0 / (1 << 12) as f64, "guess {first:e}");
        assert!(worst < 1.0 / (1 << 23) as f64, "worst {worst:e}");
    }

    /// The rasteriser's lines for `log2`'s fraction are the formula's.
    #[test]
    fn the_log_lines_are_the_formula() {
        for k in 0..32u32 {
            let at = U::<5>::from(k);
            let have = (log_start(at).raw() as u32, log_rise(at).raw() as u32);
            assert_eq!(have, log_line(k), "segment {k}");
        }
    }

    /// The rasteriser's lines for the first guess are the formula's.
    #[test]
    fn the_seed_lines_are_the_formula() {
        for k in 0..32u32 {
            let at = U::<5>::from(k);
            let have =
                (seed_start(at).raw() as u32, seed_fall(at).raw() as u32);
            assert_eq!(have, seed_line(k), "segment {k}");
        }
    }

    /// The texel coordinates are `(u q) / q` to well under a 256th of a
    /// texel, over `q` from one down to a thousandth and `u` across the
    /// range a texture of 1024 repeated takes.
    #[test]
    fn the_texel_coordinates_are_the_quotient() {
        let one = 1u64 << 48;
        for &q in &[one, one / 3, one / 17, one / 1000] {
            for &u in &[0.0f64, 1.5, -7.25, 511.75, 4095.5] {
                let qr = q as f64 / one as f64;
                let uq = (u * qr * (1u64 << 32) as f64).round() as i64 as u64;
                let (got, _) = texel_uv(uq, 0, q);
                let want = u * 256.0;
                assert!(
                    (got as f64 - want).abs() <= 1.0,
                    "u {u} q {qr}: {got}"
                );
            }
        }
    }

    /// `log2 q` to within 1.5 256ths, from one down to a thousandth: half a
    /// table's step, 0.72, and the table's own rounding, 0.5.
    #[test]
    fn log2_of_q() {
        let one = 1u64 << 48;
        for k in 1..=1000u64 {
            let q = one / k;
            let want = (q as f64 / one as f64).log2() * 256.0;
            let got = log2q(q) as f64;
            assert!((got - want).abs() <= 1.5, "1/{k}: {got} not {want}");
        }
    }

    /// The environments on an RGBA texel, against GL's table.
    #[test]
    fn the_environments_on_rgba() {
        let (cf, ct, cc) = (0x80ff_4000, 0x4000_ff80_u32, 0xff10_2030);
        assert_eq!(env(REPLACE, 0, cf, ct, cc), ct);
        let m = env(MODULATE, 0, cf, ct, cc);
        assert_eq!(m >> 24, div255(0x80 * 0x40));
        assert_eq!((m >> 16) & 0xff, 0, "red times none");
        assert_eq!(env(ADD, 0, cf, ct, cc) & 0xff, 0x80);
        assert_eq!(env(DECAL, 0, cf, ct, cc) >> 24, 0x80);
    }
}

/// The model's texturing against GL's formulas in floating point: a
/// perspective-correct, mipmapped quad, under every filter and every
/// environment.
#[cfg(test)]
mod against_gl {
    use crate::model::{render_textured, Textures};
    use crate::op::{assemble, Op, TexMode};
    use razboj_tile::tex::{
        encode, level_base, side, texel_offset, Desc, ADD, BLEND, DECAL,
        LINEAR, LINEAR_MIPMAP_LINEAR, LINEAR_MIPMAP_NEAREST, MODULATE, NEAREST,
        NEAREST_MIPMAP_LINEAR, NEAREST_MIPMAP_NEAREST, REPLACE, RGBA,
    };
    use std::collections::HashMap;

    const W: usize = 128;
    const H: usize = 128;
    const TABLE: u32 = 0x8000;
    const BASE: u32 = 0x10000;

    /// A texture of 64 by 64 with all its levels, each texel a pattern of
    /// its place and level, in a memory of words, with its descriptor.
    fn texture(min: u32, mag: u32, clamp: bool) -> (HashMap<u32, u32>, Desc) {
        let d = Desc {
            base: BASE,
            log_w: 6,
            log_h: 6,
            levels: 7,
            clamp_s: clamp,
            clamp_t: false,
            min,
            mag,
            class: RGBA,
        };
        let mut mem = HashMap::new();
        for l in 0..7 {
            let (w, h) = (side(6, l), side(6, l));
            for j in 0..h {
                for i in 0..w {
                    let c = (i * 37 + j * 11 + l * 50) & 0xff
                        | ((i * 5 + 100) & 0xff) << 8
                        | ((j * 7 + l * 30) & 0xff) << 16
                        | (128 + (((i ^ j) * 2) & 0x7f)) << 24;
                    mem.insert(
                        level_base(&d, l) + texel_offset(&d, l, i, j),
                        c,
                    );
                }
            }
        }
        for (k, w) in encode(&d).iter().enumerate() {
            mem.insert(TABLE + 4 * k as u32, *w);
        }
        (mem, d)
    }

    /// The quad: a floor going away, its four corners in the window with
    /// their clip w and their texture coordinates, two triangles.
    const CORNERS: [((f64, f64), f64, (f64, f64)); 4] = [
        ((10.0, 120.0), 1.0, (0.0, 0.0)),
        ((118.0, 120.0), 1.0, (2.0, 0.0)),
        ((90.0, 20.0), 6.0, (2.0, 3.0)),
        ((38.0, 20.0), 6.0, (0.0, 3.0)),
    ];

    /// A triangle of the quad as the assembler takes it: each vertex's
    /// `u q`, `v q` and `q`, with `q` scaled so its largest is one.
    fn tri(k: [usize; 3], colour: u32) -> Op {
        let qmax = CORNERS.iter().map(|c| 1.0 / c.1).fold(0.0, f64::max);
        let uvq = k.map(|n| {
            let ((_, _), w, (s, t)) = CORNERS[n];
            let q = 1.0 / w / qmax;
            let fx = |v: f64, b: u32| (v * (1u64 << b) as f64).round();
            (
                fx(s * 64.0 * q, 32) as i64,
                fx(t * 64.0 * q, 32) as i64,
                fx(q, 48) as u64,
            )
        });
        let p = k.map(|n| CORNERS[n].0);
        let v = |n: usize| {
            let (x, y) = p[n];
            ((x * 16.0).round() as i32, (y * 16.0).round() as i32)
        };
        Op::TexTri {
            a: v(0),
            b: v(1),
            c: v(2),
            colours: [colour; 3],
            shaded: false,
            z: [0; 3],
            uvq,
        }
    }

    /// GL's level of detail at the point `at` of the triangle `k`, with
    /// the scale factor `ρ = max(|∂u/∂x|, |∂u/∂y|, |∂v/∂x|, |∂v/∂y|)` GL
    /// allows, and its `q` scaled as the triangle's, from the derivatives
    /// of `u` and `v` across the window, which `(u q) / q` gives exactly.
    fn exact(k: [usize; 3], at: (f64, f64)) -> (f64, f64) {
        let qmax = CORNERS.iter().map(|c| 1.0 / c.1).fold(0.0, f64::max);
        let uv = |at: (f64, f64)| {
            let (b, q) = interp(k, at);
            let _ = qmax;
            (b.0 / q, b.1 / q, q)
        };
        let h = 1e-3;
        let (u, v, q) = uv(at);
        let (ux, vx, _) = uv((at.0 + h, at.1));
        let (uy, vy, _) = uv((at.0, at.1 + h));
        let rho = [(ux - u) / h, (vx - v) / h, (uy - u) / h, (vy - v) / h]
            .iter()
            .fold(0f64, |m, d| m.max(d.abs()));
        (rho.log2(), q)
    }

    /// `u q`, `v q` and `q` at a point of triangle `k`, interpolated in the
    /// window, `q` scaled so that its largest is one.
    fn interp(k: [usize; 3], at: (f64, f64)) -> ((f64, f64), f64) {
        let qmax = CORNERS.iter().map(|c| 1.0 / c.1).fold(0.0, f64::max);
        let p = k.map(|n| CORNERS[n]);
        let ((x0, y0), (x1, y1), (x2, y2)) = (p[0].0, p[1].0, p[2].0);
        let area = (x1 - x0) * (y2 - y0) - (y1 - y0) * (x2 - x0);
        let l1 = ((at.0 - x0) * (y2 - y0) - (at.1 - y0) * (x2 - x0)) / area;
        let l2 = ((x1 - x0) * (at.1 - y0) - (y1 - y0) * (at.0 - x0)) / area;
        let l0 = 1.0 - l1 - l2;
        let l = [l0, l1, l2];
        let (mut uq, mut vq, mut q) = (0.0, 0.0, 0.0);
        for n in 0..3 {
            let qn = 1.0 / p[n].1 / qmax;
            uq += l[n] * p[n].2 .0 * 64.0 * qn;
            vq += l[n] * p[n].2 .1 * 64.0 * qn;
            q += l[n] * qn;
        }
        ((uq, vq), q)
    }

    /// GL's sampling in floating point: the level or levels `λ` picks, the
    /// filter, and the wrap, from the texels in memory.
    fn gl_sample(
        d: &Desc,
        mem: &HashMap<u32, u32>,
        u: f64,
        v: f64,
        lam: f64,
    ) -> [f64; 4] {
        let texel = |l: u32, i: i64, j: i64| {
            let n = side(6, l) as i64;
            let i = if d.clamp_s {
                i.clamp(0, n - 1)
            } else {
                i.rem_euclid(n)
            };
            let j = j.rem_euclid(n);
            let w = mem
                [&(level_base(d, l) + texel_offset(d, l, i as u32, j as u32))];
            [0, 8, 16, 24].map(|at| ((w >> at) & 0xff) as f64)
        };
        let level = |l: u32, f: u32| {
            let s = (1u32 << l) as f64;
            let (u, v) = (u / s, v / s);
            if f == NEAREST {
                return texel(l, u.floor() as i64, v.floor() as i64);
            }
            let (u, v) = (u - 0.5, v - 0.5);
            let (i, j) = (u.floor() as i64, v.floor() as i64);
            let (a, b) = (u - u.floor(), v - v.floor());
            let t = [
                texel(l, i, j),
                texel(l, i + 1, j),
                texel(l, i, j + 1),
                texel(l, i + 1, j + 1),
            ];
            let w =
                [(1.0 - a) * (1.0 - b), a * (1.0 - b), (1.0 - a) * b, a * b];
            std::array::from_fn(|c| (0..4).map(|k| w[k] * t[k][c]).sum())
        };
        let c = if d.mag == LINEAR
            && (d.min == NEAREST_MIPMAP_NEAREST
                || d.min == NEAREST_MIPMAP_LINEAR)
        {
            0.5
        } else {
            0.0
        };
        if lam <= c {
            return level(0, d.mag);
        }
        let top = 6.0;
        match d.min {
            NEAREST | LINEAR => level(0, d.min),
            NEAREST_MIPMAP_NEAREST | LINEAR_MIPMAP_NEAREST => {
                let f = if d.min == NEAREST_MIPMAP_NEAREST {
                    NEAREST
                } else {
                    LINEAR
                };
                let l = if lam <= 0.5 {
                    0.0
                } else {
                    ((lam + 0.5).ceil() - 1.0).min(top)
                };
                level(l as u32, f)
            }
            _ => {
                let f = if d.min == NEAREST_MIPMAP_LINEAR {
                    NEAREST
                } else {
                    LINEAR
                };
                if lam >= top {
                    return level(top as u32, f);
                }
                let l = lam.floor();
                let (t1, t2) = (level(l as u32, f), level(l as u32 + 1, f));
                let fr = lam - l;
                std::array::from_fn(|c| (1.0 - fr) * t1[c] + fr * t2[c])
            }
        }
    }

    /// GL's environments on an RGBA texel, in floating point, channels
    /// blue first.
    fn gl_env(mode: u32, f: [f64; 4], t: [f64; 4], e: [f64; 4]) -> [f64; 4] {
        let m = |a: f64, b: f64| a * b / 255.0;
        std::array::from_fn(|c| match (mode, c) {
            (REPLACE, _) => t[c],
            (MODULATE, _) => m(f[c], t[c]),
            (DECAL, 3) => f[3],
            (DECAL, _) => (f[c] * (255.0 - t[3]) + t[c] * t[3]) / 255.0,
            (BLEND, 3) => m(f[3], t[3]),
            (BLEND, _) => (f[c] * (255.0 - t[c]) + e[c] * t[c]) / 255.0,
            (_, 3) => m(f[3], t[3]),
            _ => (f[c] + t[c]).min(255.0),
        })
    }

    /// Every filter, every environment and both wraps: each pixel of the
    /// model's picture is GL's within two in every channel at the model's
    /// level of detail, wherever GL's answer does not turn on less than
    /// the model's arithmetic resolves: near an edge of the quad, near a
    /// texel's edge under the nearest filter, or near a level's edge. The
    /// model's level of detail, from the four numerators less `2 log2 q`,
    /// is
    /// GL's own within a fiftieth of a level over the whole quad.
    #[test]
    fn the_model_samples_as_gl_does() {
        let frag = 0xc080_40ffu32;
        let env_colour = 0xff20_c060u32;
        let filters = [
            (NEAREST, NEAREST),
            (LINEAR, LINEAR),
            (NEAREST_MIPMAP_NEAREST, LINEAR),
            (LINEAR_MIPMAP_NEAREST, NEAREST),
            (NEAREST_MIPMAP_LINEAR, NEAREST),
            (LINEAR_MIPMAP_LINEAR, LINEAR),
        ];
        let (mut compared, mut skipped, mut worst_lod) = (0, 0, 0f64);
        for (k, &(min, mag)) in filters.iter().enumerate() {
            for env in [REPLACE, MODULATE, DECAL, BLEND, ADD] {
                let (mem, d) = texture(min, mag, k % 2 == 1);
                let ops = [
                    Op::Texture(Some(TexMode {
                        desc: TABLE,
                        env,
                        env_colour,
                    })),
                    tri([0, 1, 2], frag),
                    tri([0, 2, 3], frag),
                ];
                let list = assemble(&ops, W, H);
                assert!(list.iter().all(|i| i.tex.to_bool()));
                let read = |a: u32| *mem.get(&a).unwrap_or(&0);
                let t = Textures { mem: &read };
                let got =
                    render_textured(&list, W, H, vec![0; W * H], Some(&t));
                for y in 0..H {
                    for x in 0..W {
                        let at = (x as f64 + 0.5, y as f64 + 0.5);
                        let Some(kk) = [[0, 1, 2], [0, 2, 3]]
                            .into_iter()
                            .find(|k| inside(*k, at, 1.0))
                        else {
                            skipped += 1;
                            continue;
                        };
                        let ((uq, vq), q) = interp(kk, at);
                        let (u, v) = (uq / q, vq / q);
                        let (lam, _) = exact(kk, at);
                        // The model's level of detail, for the skip below.
                        let e = &list[if kk[1] == 1 { 0 } else { 1 }];
                        let (i, j) = (
                            x as i32 - e.x0.raw() as i32,
                            y as i32 - e.y0.raw() as i32,
                        );
                        let qm = (e.q0.raw() as u64)
                            .wrapping_add(
                                (e.qdx.raw() as u64).wrapping_mul(i as u64),
                            )
                            .wrapping_add(
                                (e.qdy.raw() as u64).wrapping_mul(j as u64),
                            );
                        let n = super::numerators(e, i, j);
                        let ml = super::lod(n, e.lodk.raw() as u32, qm) as f64
                            / 256.0;
                        worst_lod = worst_lod.max((ml - lam).abs());
                        let near = |x: f64, step: f64, off: f64| {
                            ((x - off) / step - ((x - off) / step).round())
                                .abs()
                                * step
                        };
                        let fuzzy = (min == NEAREST_MIPMAP_NEAREST
                            || min == LINEAR_MIPMAP_NEAREST)
                            && near(ml, 1.0, 0.5) < 0.02
                            || near(ml, 1.0, 0.0) < 0.02
                            || (mag == NEAREST
                                || min != LINEAR
                                    && min != LINEAR_MIPMAP_LINEAR
                                    && min != LINEAR_MIPMAP_NEAREST)
                                && (near(u, 1.0, 0.0)
                                    < 0.05
                                        * (1u32 << (ml.max(0.0) as u32 + 1))
                                            as f64
                                    || near(v, 1.0, 0.0)
                                        < 0.05
                                            * (1u32 << (ml.max(0.0) as u32 + 1))
                                                as f64);
                        if fuzzy {
                            skipped += 1;
                            continue;
                        }
                        let t = gl_sample(&d, &mem, u, v, ml);
                        let f = [0, 8, 16, 24]
                            .map(|at| ((frag >> at) & 0xff) as f64);
                        let e = [0, 8, 16, 24]
                            .map(|at| ((env_colour >> at) & 0xff) as f64);
                        let want = gl_env(env, f, t, e);
                        let p = got[y * W + x];
                        for (c, w) in want.iter().enumerate() {
                            let g = ((p >> (8 * c)) & 0xff) as f64;
                            assert!(
                                (g - w).abs() <= 2.5,
                                "min {min} mag {mag} env {env} at ({x}, {y}) \
                                 channel {c}: \
                                 {g}, GL {:.2}; u {u:.3} v {v:.3} λ {lam:.3}",
                                w
                            );
                        }
                        compared += 1;
                    }
                }
            }
        }
        println!(
            "{compared} compared, {skipped} skipped, \
             λ off by {worst_lod:.3} at most"
        );
        assert!(
            compared > 30 * W * H / 2 / 2,
            "{compared} compared, {skipped} skipped"
        );
        assert!(worst_lod < 0.02, "λ off by {worst_lod:.3}");
    }

    /// A textured list survives the format and the binning: its words read
    /// back draw the same picture, and so do its entries binned into
    /// tiles, whose planes and numerators are stepped to each tile.
    #[test]
    fn a_textured_list_survives_the_format_and_the_tiles() {
        let (mem, _) = texture(LINEAR_MIPMAP_LINEAR, LINEAR, false);
        let ops = [
            Op::Texture(Some(TexMode {
                desc: TABLE,
                env: MODULATE,
                env_colour: 0,
            })),
            tri([0, 1, 2], 0xffff_ffff),
            tri([0, 2, 3], 0xffc0_c0c0),
        ];
        let list = assemble(&ops, W, H);
        let read = |a: u32| *mem.get(&a).unwrap_or(&0);
        let t = Textures { mem: &read };
        let draw = |l: &[crate::op::Insn]| {
            render_textured(l, W, H, vec![0; W * H], Some(&t))
        };
        let want = draw(&list);
        let words: Vec<[u32; 16]> = crate::dl::image(&list)
            .chunks(16)
            .map(|c| c.try_into().unwrap())
            .collect();
        assert_eq!(words.len(), 2 * 4, "each entry four slots");
        assert_eq!(draw(&crate::dl::decode_list(&words)), want);
        let tiled = crate::tiles::tiled(&list, W, H);
        assert!(tiled.tiles.len() >= 4);
        assert_eq!(draw(&tiled.entries), want, "binned into tiles");
        assert!(want.iter().filter(|&&p| p != 0).count() > W * H / 3);
    }

    /// Whether a point is inside triangle `k` by more than `margin` pixels.
    fn inside(k: [usize; 3], at: (f64, f64), margin: f64) -> bool {
        let p = k.map(|n| CORNERS[n].0);
        let area = (p[1].0 - p[0].0) * (p[2].1 - p[0].1)
            - (p[1].1 - p[0].1) * (p[2].0 - p[0].0);
        (0..3).all(|e| {
            let (a, b) = (p[e], p[(e + 1) % 3]);
            let d = ((b.0 - a.0) * (at.1 - a.1) - (b.1 - a.1) * (at.0 - a.0))
                * area.signum();
            d / (b.0 - a.0).hypot(b.1 - a.1) > margin
        })
    }
}
