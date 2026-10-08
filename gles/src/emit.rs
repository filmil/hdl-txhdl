// SPDX-License-Identifier: Apache-2.0
//! A triangle in Razboj's window space as the instruction's words:
//! what `razboj::op::Op::encode` does for `TriQ4` (#988) and `Gouraud`
//! (#989), written again here without the standard library, with the
//! planes in 64 bits rather than 128. The words are those of
//! `razboj::dl`'s format, and the tests check them against it, bit for
//! bit.
//!
//! The planes fit in 64 bits for vertices in Razboj's range
//! (`docs/gles.md`, section 4): a difference of two vertices is under
//! 2^15 sixteenths, a channel's difference under 2^8, so a gradient's
//! numerator is under 2^24, its products with an offset inside the box
//! under 2^39, their sum under 2^40, and that times 2^16 under 2^56;
//! twice the area is under 2^31.

use razboj_tile::{clip, Bounds, WORDS};

/// A vertex's coordinates, in sixteenths of a pixel: the range Razboj
/// takes, 1024 pixels either side of the origin.
pub const VMIN: i32 = -1024 * 16;
pub const VMAX: i32 = 1024 * 16 - 1;

/// Sixteenths of a pixel.
const SUB: i64 = 16;
const SUB_BITS: u32 = SUB.trailing_zeros();

/// Twice the signed area of `a`, `b`, `c`: positive when they are wound
/// the way the rasteriser wants, clockwise on a screen whose y grows
/// downwards.
pub fn area2(a: (i32, i32), b: (i32, i32), c: (i32, i32)) -> i64 {
    let (bx, by) = ((b.0 - a.0) as i64, (b.1 - a.1) as i64);
    let (cx, cy) = ((c.0 - a.0) as i64, (c.1 - a.1) as i64);
    bx * cy - by * cx
}

/// One channel's plane over a triangle wound as the rasteriser wants,
/// with values `v` at its vertices: its value at `first`, and its two
/// steps, each with `frac` bits of fraction, sixteen for a colour and
/// [`ZFRAC`] for a depth, rounded to the nearest, the start a half unit
/// up so that the value the rasteriser takes above the fraction is the
/// nearest one. The same numbers `op.rs`'s `plane` gets in 128 bits; a
/// depth's, under 2^16 at a vertex, fits 64 bits as a channel's does,
/// since it has four bits fewer of fraction for its eight more of value.
fn plane(
    a: (i32, i32),
    b: (i32, i32),
    c: (i32, i32),
    v: [i64; 3],
    first: (i32, i32),
    frac: u32,
) -> (u32, u32, u32) {
    let d =
        |p: (i32, i32), q: (i32, i32)| ((q.0 - p.0) as i64, (q.1 - p.1) as i64);
    let ((ux, uy), (vx, vy)) = (d(a, b), d(a, c));
    let area = ux * vy - uy * vx;
    let (db, dc) = (v[1] - v[0], v[2] - v[0]);
    let nx = db * vy - dc * uy;
    let ny = dc * ux - db * vx;
    let one = 1i64 << frac;
    let round = |n: i64| (n + area / 2).div_euclid(area);
    let (px, py) = d(a, first);
    let start = v[0] * one + one / 2 + round((nx * px + ny * py) * one);
    let word = |x: i64| x as i32 as u32;
    (
        word(start),
        word(round(nx * SUB * one)),
        word(round(ny * SUB * one)),
    )
}

/// Bits of fraction in a depth plane, `razboj::op::ZFRAC`: sixteen bits
/// of depth and the sign in thirty-two.
pub const ZFRAC: u32 = 12;

/// The farthest depth, where every tile's depth starts (#992).
pub const FAR: u32 = 0xffff;

/// An instruction's word 15 told to test depth with `func`, Razboj's
/// comparison from nought for `GL_NEVER` to seven for `GL_ALWAYS`, and
/// to write its depth where it passes if `write`: the instruction then
/// takes its depth plane's slot after it.
pub fn depth(w: &mut [u32; WORDS], func: u32, write: bool) {
    w[15] = (w[15] & 0xff) | 1 << 8 | (func & 7) << 9 | (write as u32) << 12;
}

/// What happens to a pixel after its coverage (#993), as Razboj's list
/// says it: the blend's two factors, Razboj's codes from nought for
/// `GL_ZERO`; the alpha test's comparison, from nought for `GL_NEVER`,
/// and its reference, a byte; and the channels written, a bit a byte,
/// bit 0 blue to bit 3 alpha.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Pixel {
    pub blend: Option<(u32, u32)>,
    pub alpha: Option<(u32, u32)>,
    pub mask: u32,
}

impl Pixel {
    /// GL's initial state, which an instruction says by leaving it out.
    pub const DEFAULT: Pixel = Pixel {
        blend: None,
        alpha: None,
        mask: 0xf,
    };
}

/// An instruction's word 15 told it has the pixel's state `p`, and its
/// second slot, `slot`, given it in words 3 and 4 (Razboj's #993).
pub fn state(w: &mut [u32; WORDS], slot: &mut [u32; WORDS], p: Pixel) {
    w[15] |= 1 << 13;
    slot[3] = p
        .blend
        .map_or(0, |(s, d)| 1 | (s & 0xf) << 4 | (d & 0xf) << 8);
    slot[4] = p
        .alpha
        .map_or(0, |(f, r)| 1 | (f & 7) << 1 | (r & 0xff) << 8)
        | (p.mask & 0xf) << 16;
}

/// A depth plane's slot for a depth `z` everywhere, of sixteen bits:
/// the plane of a clear, a rectangle or a point, half a unit up as a
/// triangle's start is.
pub fn flat_depth(z: u32) -> [u32; WORDS] {
    let mut w = [0u32; WORDS];
    w[0] = ((z & 0xffff) << ZFRAC) + (1 << (ZFRAC - 1));
    w
}

/// A triangle on a screen `within`, its vertices in sixteenths, flat in
/// `colour` or, with `shades`, a colour at each vertex blended across
/// it; a colour is `0xAARRGGBB`, and a shaded triangle's alpha is its
/// first vertex's. With `zs`, a depth of sixteen bits at each vertex,
/// the triangle's depth plane's slot comes with it, for [`depth`] to
/// make the instruction test. `None` when there is nothing to draw: no
/// area, a vertex out of Razboj's range, or a box off the screen. The
/// winding is the rasteriser's, two vertices swapped with their colours
/// and depths when the area says the other way.
pub fn triangle(
    colour: u32,
    a: (i32, i32),
    b: (i32, i32),
    c: (i32, i32),
    shades: Option<[u32; 3]>,
    zs: Option<[u32; 3]>,
    within: Bounds,
) -> Option<([u32; WORDS], Option<[u32; WORDS]>)> {
    let swap = area2(a, b, c) < 0;
    let (b, c) = if swap { (c, b) } else { (b, c) };
    let turn = |s: [u32; 3]| if swap { [s[0], s[2], s[1]] } else { s };
    let (shades, zs) = (shades.map(turn), zs.map(turn));
    if area2(a, b, c) == 0 {
        return None;
    }
    let ok = |p: (i32, i32)| {
        (VMIN..=VMAX).contains(&p.0) && (VMIN..=VMAX).contains(&p.1)
    };
    if !ok(a) || !ok(b) || !ok(c) {
        return None;
    }
    let px = |v: i32| v.div_euclid(SUB as i32);
    let lo = |f: fn((i32, i32)) -> i32| px(f(a).min(f(b)).min(f(c)));
    let hi = |f: fn((i32, i32)) -> i32| px(f(a).max(f(b)).max(f(c)));
    let (x0, y0, x1, y1) =
        clip(lo(|p| p.0), lo(|p| p.1), hi(|p| p.0), hi(|p| p.1), within)?;
    let pair =
        |p: (i32, i32)| (p.0 as u32 & 0xffff) | ((p.1 as u32 & 0xffff) << 16);
    let mut w = [0u32; WORDS];
    w[0] = 2 | ((colour & 0xff_ffff) << 2);
    w[1] = x0 | (y0 << 16);
    w[2] = x1 | (y1 << 16);
    w[3] = pair(a);
    w[4] = pair(b);
    w[5] = pair(c);
    w[15] = colour >> 24;
    let first = (x0 as i32 * 16 + 8, y0 as i32 * 16 + 8);
    if let Some(s) = shades {
        w[0] = 3 | ((colour & 0xff_ffff) << 2);
        for (k, at) in [16u32, 8, 0].iter().enumerate() {
            let ch = |v: u32| ((v >> at) & 0xff) as i64;
            let (s0, dx, dy) =
                plane(a, b, c, [ch(s[0]), ch(s[1]), ch(s[2])], first, 16);
            w[6 + 3 * k] = s0;
            w[7 + 3 * k] = dx;
            w[8 + 3 * k] = dy;
        }
    }
    let slot = zs.map(|z| {
        let v = z.map(|z| (z & 0xffff) as i64);
        let (z0, dx, dy) = plane(a, b, c, v, first, ZFRAC);
        let mut p = [0u32; WORDS];
        (p[0], p[1], p[2]) = (z0, dx, dy);
        p
    });
    Some((w, slot))
}

/// A clear of the whole screen in `colour`, `0xAARRGGBB`.
pub fn clear(colour: u32) -> [u32; WORDS] {
    let mut w = [0u32; WORDS];
    w[0] = (colour & 0xff_ffff) << 2;
    w[15] = colour >> 24;
    w
}

/// A rectangle filled in `colour`, `0xAARRGGBB`, over `within`, both
/// ends included: the clear of a window that does not start at row zero
/// (issue 996).
pub fn rect(colour: u32, within: Bounds) -> [u32; WORDS] {
    let (x0, y0, x1, y1) = within;
    let mut w = [0u32; WORDS];
    w[0] = 1 | ((colour & 0xff_ffff) << 2);
    w[1] = x0 | (y0 << 16);
    w[2] = x1 | (y1 << 16);
    w[15] = colour >> 24;
    w
}

/// A triangle's two texture slots (#997), for the triangle `v`, in
/// sixteenths, on `within`, with `u q`, `v q` and `q` at its vertices
/// (32, 32 and 48 bits of fraction): its three planes and the level of
/// detail's numerators, as `razboj::op`'s assembler works them out, bit
/// for bit, and the tests check. Words 13 to 15 of the first slot, the
/// texture and its environment, are the caller's. `None` where
/// [`triangle`] draws nothing.
pub fn textured(
    v: [(i32, i32); 3],
    uvq: [(i64, i64, u64); 3],
    within: Bounds,
) -> Option<[[u32; WORDS]; 2]> {
    let [a, b, c] = v;
    let swap = area2(a, b, c) < 0;
    let (b, c) = if swap { (c, b) } else { (b, c) };
    let t = if swap { [uvq[0], uvq[2], uvq[1]] } else { uvq };
    if area2(a, b, c) == 0 {
        return None;
    }
    let px = |v: i32| v.div_euclid(SUB as i32);
    let lo = |f: fn((i32, i32)) -> i32| px(f(a).min(f(b)).min(f(c)));
    let hi = |f: fn((i32, i32)) -> i32| px(f(a).max(f(b)).max(f(c)));
    let (x0, y0, x1, y1) =
        clip(lo(|p| p.0), lo(|p| p.1), hi(|p| p.0), hi(|p| p.1), within)?;
    let first = (x0 as i32 * 16 + 8, y0 as i32 * 16 + 8);
    let r = recip(a, b, c);
    let pl =
        |v: [i128; 3]| plane64(a, b, c, v, first, r).map(|v| v as i64 as i128);
    let u = pl(t.map(|t| t.0 as i128));
    let w = pl(t.map(|t| t.1 as i128));
    let q = pl(t.map(|t| t.2 as i128));
    let ([u0, ux, uy], [v0, vx, vy], [q0, qx, qy]) = (u, w, q);
    let n = [
        (ux * q0 - u0 * qx, ux * qy - uy * qx),
        (vx * q0 - v0 * qx, vx * qy - vy * qx),
        (uy * q0 - u0 * qy, uy * qx - ux * qy),
        (vy * q0 - v0 * qy, vy * qx - vx * qy),
    ];
    let (bw, bh) = ((x1 - x0) as i128, (y1 - y0) as i128);
    let far = n
        .iter()
        .enumerate()
        .map(|(k, &(v, d))| {
            let span = if k < 2 { bh } else { bw };
            v.abs().max((v + d * span).abs())
        })
        .max()
        .unwrap_or(0);
    let k = (128 - far.leading_zeros()).saturating_sub(30);
    let (mut sa, mut sb) = ([0u32; WORDS], [0u32; WORDS]);
    let words =
        |p: [i128; 3]| razboj_tile::tex::plane_words(p.map(|v| v as u64));
    sa[0..6].copy_from_slice(&words(u));
    sa[6..12].copy_from_slice(&words(w));
    sa[12] = k;
    sb[0..6].copy_from_slice(&words(q));
    for (j, (v, d)) in n.iter().enumerate() {
        sb[6 + 2 * j] = (v >> k) as i32 as u32;
        sb[7 + 2 * j] = (d >> k) as i32 as u32;
    }
    Some([sa, sb])
}

/// A plane in 64 bits for a texture: its value at `first` and its two
/// steps, in the units of the values `v` at the vertices, rounded to the
/// nearest, as `razboj::op`'s `plane64`.
///
/// Nothing is divided (#1433): `r` is the reciprocal of the area, which
/// [`recip`] works out once for the three planes, and the differences
/// between the vertices keep their top 46 bits, so that the products
/// with the vertices' offsets fit 64 bits.
fn plane64(
    a: (i32, i32),
    b: (i32, i32),
    c: (i32, i32),
    v: [i128; 3],
    first: (i32, i32),
    r: (i128, u32),
) -> [i128; 3] {
    let d =
        |p: (i32, i32), q: (i32, i32)| ((q.0 - p.0) as i64, (q.1 - p.1) as i64);
    let ((ux, uy), (vx, vy)) = (d(a, b), d(a, c));
    let (db, dc) = (v[1] - v[0], v[2] - v[0]);
    let big = db.unsigned_abs().max(dc.unsigned_abs());
    let s = (128 - big.leading_zeros()).saturating_sub(46);
    let (db, dc) = ((db >> s) as i64, (dc >> s) as i64);
    let nx = db * vy - dc * uy;
    let ny = dc * ux - db * vx;
    // A pixel's steps with sixteen more bits of fraction, for the start.
    let (m, sh) = r;
    let step = |n: i64| shr(n as i128 * m, sh - SUB_BITS - 16);
    let (fx, fy) = (step(nx), step(ny));
    let (px, py) = d(a, first);
    let at = shr(fx * px as i128 + fy * py as i128, SUB_BITS + 16);
    [v[0] + (at << s), shr(fx, 16) << s, shr(fy, 16) << s]
}

/// The reciprocal of a triangle's area, twice it in sixteenths squared,
/// for [`plane64`]: `m` over two to the `sh` is within one part in
/// `2^62` of one over the area, and `m` is at most `2^63`. Two divisions
/// of 64 bits, the first of the top and the second of what is left.
fn recip(a: (i32, i32), b: (i32, i32), c: (i32, i32)) -> (i128, u32) {
    let area = area2(a, b, c) as u64;
    let l = 64 - area.leading_zeros();
    let top = 1u64 << (30 + l);
    let (q, rest) = (top / area, top % area);
    (((q << 32) + (rest << 32) / area) as i128, 62 + l)
}

/// `x` over two to the `k`, rounded to the nearest, a half up.
fn shr(x: i128, k: u32) -> i128 {
    (x + (1 << (k - 1))) >> k
}

/// An instruction's word 15 told it is textured, which gives it its
/// second slot and the two texture slots after that.
pub fn textured_bit(w: &mut [u32; WORDS]) {
    w[15] |= razboj_tile::tex::TEXTURED;
}
