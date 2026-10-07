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
