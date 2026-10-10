// SPDX-License-Identifier: Apache-2.0
//! The rasteriser written with loops: the rule the hardware is
//! checked against. It decodes the same instructions and tests the
//! same edge functions, but it says them as a program rather than as
//! a step per cycle, so that the two agreeing means something.
use crate::op::{
    signed, Insn, Kind, DST_ALPHA, DST_COLOR, EQUAL, GEQUAL, GREATER, LEQUAL,
    LESS, NEVER, NOTEQUAL, ONE, ONE_MINUS_DST_ALPHA, ONE_MINUS_DST_COLOR,
    ONE_MINUS_SRC_ALPHA, ONE_MINUS_SRC_COLOR, SRC_ALPHA, SRC_ALPHA_SATURATE,
    SRC_COLOR, SUB, ZERO, ZFRAC,
};
use txhdl::types::U;

/// An edge function of the edge from `a` to `b`, at `p`, all in
/// sixteenths of a pixel. Positive on one side, negative on the other,
/// zero on the edge.
fn edge(a: (i32, i32), b: (i32, i32), p: (i32, i32)) -> i64 {
    let (bx, by) = ((b.0 - a.0) as i64, (b.1 - a.1) as i64);
    let (px, py) = ((p.0 - a.0) as i64, (p.1 - a.1) as i64);
    bx * py - by * px
}

/// Whether the edge from `a` to `b` is a top or a left edge of a
/// triangle wound as the rasteriser wants: inside is to its right, or
/// it is level and inside is below it. Its function grows to the right,
/// or does not change across and grows downwards.
fn top_left(a: (i32, i32), b: (i32, i32)) -> bool {
    let (ddx, ddy) = (-(b.1 - a.1), b.0 - a.0);
    ddx > 0 || (ddx == 0 && ddy > 0)
}

/// Whether a pixel is in the entry: every pixel of a clear or a
/// rectangle, and of a triangle the pixels whose centre is inside it.
/// A centre exactly on an edge is inside only if the edge is a top or
/// a left one, so that of two triangles sharing an edge, exactly one
/// draws a pixel on it: the top-left rule.
fn inside(op: &Insn, x: i32, y: i32) -> bool {
    if op.kind != Kind::Tri && op.kind != Kind::Shaded {
        return true;
    }
    let v = |x, y| (signed(x), signed(y));
    let (a, b, c) = (v(op.ax, op.ay), v(op.bx, op.by), v(op.cx, op.cy));
    let p = (x * SUB + SUB / 2, y * SUB + SUB / 2);
    let holds = |a, b| {
        let e = edge(a, b, p);
        e > 0 || (e == 0 && top_left(a, b))
    };
    holds(a, b) && holds(b, c) && holds(c, a)
}

/// How many entries of a list cover each pixel, for a test that every
/// pixel is drawn exactly once.
pub fn coverage(ops: &[Insn], w: usize, h: usize) -> Vec<u32> {
    let mut n = vec![0u32; w * h];
    for op in ops {
        let (x0, y0, x1, y1) = box_of(op, w, h);
        for y in y0..=y1 {
            for x in x0..=x1 {
                if inside(op, x, y) {
                    n[y as usize * w + x as usize] += 1;
                }
            }
        }
    }
    n
}

/// The box an instruction walks. A clear's is the whole screen, which
/// the instruction does not carry and the rasteriser supplies.
fn box_of(op: &Insn, w: usize, h: usize) -> (i32, i32, i32, i32) {
    if op.kind == Kind::Clear {
        (0, 0, w as i32 - 1, h as i32 - 1)
    } else {
        (
            op.x0.raw() as i32,
            op.y0.raw() as i32,
            op.x1.raw() as i32,
            op.y1.raw() as i32,
        )
    }
}

/// One channel of a plane, `i` pixels right and `j` rows down of the
/// box's first pixel: its start and that many of each step, in the
/// thirty-two bits the rasteriser's adders wrap in, then clamped to a
/// byte. A value below nought is nought, one of 256 or more is 255.
pub(crate) fn channel(start: u32, dx: u32, dy: u32, i: i32, j: i32) -> u32 {
    let v = start
        .wrapping_add(dx.wrapping_mul(i as u32))
        .wrapping_add(dy.wrapping_mul(j as u32));
    if v & 0x8000_0000 != 0 {
        0
    } else if v >> 24 != 0 {
        255
    } else {
        (v >> 16) & 0xff
    }
}

/// The word an entry writes at the pixel `i` right and `j` down of its
/// box's first: its alpha above its colour, which is its own or a
/// shaded triangle's three planes there.
fn colour(op: &Insn, i: i32, j: i32) -> u32 {
    let p = |a: U<32>, b: U<32>, c: U<32>| {
        channel(a.raw() as u32, b.raw() as u32, c.raw() as u32, i, j)
    };
    // A shaded triangle's alpha plane where it has one (issue 1520),
    // which only the pixel's state carries.
    let alpha = if op.state.to_bool() && op.ashade.to_bool() {
        p(op.a0, op.adx, op.ady) << 24
    } else {
        (op.alpha.raw() as u32) << 24
    };
    if op.kind != Kind::Shaded {
        return alpha | op.colour.raw() as u32;
    }
    let r = p(op.r0, op.rdx, op.rdy);
    let g = p(op.g0, op.gdx, op.gdy);
    let b = p(op.b0, op.bdx, op.bdy);
    alpha | (r << 16) | (g << 8) | b
}

/// The depth plane at the pixel `i` right and `j` down of the box's
/// first, as the rasteriser steps it (issue 992): wrapping in thirty-two
/// bits, then the sixteen bits above the fraction, nought below nought
/// and the farthest, `0xffff`, past it.
pub(crate) fn depth(start: u32, dx: u32, dy: u32, i: i32, j: i32) -> u32 {
    let v = start
        .wrapping_add(dx.wrapping_mul(i as u32))
        .wrapping_add(dy.wrapping_mul(j as u32));
    if v & 0x8000_0000 != 0 {
        0
    } else {
        (v >> ZFRAC).min(0xffff)
    }
}

/// Whether a pixel at depth `z` passes the comparison `func`, GL's, with
/// `d` the depth already there.
pub fn passes(func: u32, z: u32, d: u32) -> bool {
    match func {
        NEVER => false,
        LESS => z < d,
        EQUAL => z == d,
        LEQUAL => z <= d,
        GREATER => z > d,
        NOTEQUAL => z != d,
        GEQUAL => z >= d,
        _ => true,
    }
}

/// `x` over 255, rounded to the nearest, for `x` up to 65535, and 255
/// or more past it: what the rasteriser's blend does without a divider.
pub fn div255(x: u32) -> u32 {
    let y = x + 128;
    (y + (y >> 8)) >> 8
}

/// One blend factor for the channel `c` of a pixel, a byte, from the
/// source `s` and the destination `d`, each `0xAARRGGBB`, `c` being the
/// channel's byte, 0 for blue to 3 for alpha (issue 993).
pub fn factor(f: u32, s: u32, d: u32, c: u32) -> u32 {
    let ch = |p: u32| (p >> (8 * c)) & 0xff;
    let (sa, da) = (s >> 24, d >> 24);
    match f {
        ZERO => 0,
        ONE => 255,
        SRC_COLOR => ch(s),
        ONE_MINUS_SRC_COLOR => 255 - ch(s),
        SRC_ALPHA => sa,
        ONE_MINUS_SRC_ALPHA => 255 - sa,
        DST_ALPHA => da,
        ONE_MINUS_DST_ALPHA => 255 - da,
        DST_COLOR => ch(d),
        ONE_MINUS_DST_COLOR => 255 - ch(d),
        SRC_ALPHA_SATURATE if c == 3 => 255,
        SRC_ALPHA_SATURATE => sa.min(255 - da),
        _ => 0,
    }
}

/// The source `s` blended over the destination `d` with the factors
/// `sf` and `df`: each channel `s Fs + d Fd`, over 255, at most 255.
pub fn blend(s: u32, d: u32, sf: u32, df: u32) -> u32 {
    (0..4)
        .map(|c| {
            let ch = |p: u32| (p >> (8 * c)) & 0xff;
            let x = ch(s) * factor(sf, s, d, c) + ch(d) * factor(df, s, d, c);
            div255(x).min(255) << (8 * c)
        })
        .fold(0, |a, b| a | b)
}

/// The colour `c`, `0xAARRGGBB`, fogged by the factor `f`, a byte, toward
/// the fog's colour `fc`, `0xRRGGBB` (issue 998): red, green and blue each
/// `f c + (255 - f) fc` over 255, as GL's `f C + (1 - f) C_f`, and the
/// alpha as it was.
pub fn fog(c: u32, f: u32, fc: u32) -> u32 {
    (0..3)
        .map(|k| {
            let ch = |p: u32| (p >> (8 * k)) & 0xff;
            div255(ch(c) * f + ch(fc) * (255 - f)) << (8 * k)
        })
        .fold(c & 0xff00_0000, |a, b| a | b)
}

/// The channels of `new` that `mask` holds, a bit a byte, over `old`.
pub fn masked(new: u32, old: u32, mask: u32) -> u32 {
    let m = (0..4)
        .filter(|c| (mask >> c) & 1 == 1)
        .fold(0u32, |a, c| a | (0xff << (8 * c)));
    (new & m) | (old & !m)
}

/// A display list rendered into a framebuffer of `w` by `h` pixels, as a
/// tiled list draws it: an entry that tests depth (issue 992) writes a
/// pixel only where it passes against the depth there, which starts at
/// the farthest, and writes its own depth there if it says so. A flat
/// list draws its depth entries without the test; see `crate::dl`.
pub fn render(ops: &[Insn], w: usize, h: usize) -> Vec<u32> {
    render_over(ops, w, h, vec![0u32; w * h])
}

/// The same over a framebuffer that already holds `fb`. Each pixel goes
/// through GL's steps in GL's order (issue 993): fog (issue 998), the
/// alpha test, the stencil test (issue 998), the depth test, the blend
/// with the colour there or the logic operation, and the colour mask. The
/// stencil starts at nought, and changes as the stencil's operations say
/// whether or not the pixel is written.
/// A pixel that fails a test writes nothing, its depth included.
pub fn render_over(ops: &[Insn], w: usize, h: usize, fb: Vec<u32>) -> Vec<u32> {
    render_textured(ops, w, h, fb, None)
}

/// Where a textured list's textures are (issue 997): the memory, a word
/// at each byte address a multiple of four, which holds each textured
/// entry's descriptor at the address the entry names, and the texels.
pub struct Textures<'a> {
    pub mem: &'a dyn Fn(u32) -> u32,
}

/// The same with textures: a textured entry's colour at a pixel is its
/// fragment's colour through its texture's environment, before the alpha
/// test, as GL orders it. A textured entry with no textures given draws
/// untextured.
pub fn render_textured(
    ops: &[Insn],
    w: usize,
    h: usize,
    fb: Vec<u32>,
    textures: Option<&Textures>,
) -> Vec<u32> {
    let mut fb = fb;
    let mut zb = vec![0xffffu32; w * h];
    // The stencil (issue 998), nought at first, as a tile's is.
    let mut sb = vec![0u32; w * h];
    for op in ops {
        let (x0, y0, x1, y1) = box_of(op, w, h);
        let on = op.depth.to_bool();
        let raw = |u: U<32>| u.raw() as u32;
        let func = op.zfunc.raw() as u32;
        let state = op.state.to_bool();
        let atest = state && op.atest.to_bool();
        let blending = state && op.blend.to_bool();
        let fogged = state && op.fog.to_bool();
        let sten = state && op.sten.to_bool();
        let (sref, smask) = (op.sref.raw() as u32, op.smask.raw() as u32);
        let swmask = op.swmask.raw() as u32;
        let logic =
            (state && op.logic.to_bool()).then_some(op.lop.raw() as u32);
        let (sf, df) = (op.sfactor.raw() as u32, op.dfactor.raw() as u32);
        let mask = op.mask();
        let tex = textures.filter(|_| op.tex.to_bool()).map(|t| {
            let d = op.tdesc.raw() as u32;
            let words = core::array::from_fn(|k| (t.mem)(d + 4 * k as u32));
            (t, razboj_tile::tex::decode(&words))
        });
        let r64 = |u: U<64>| u.raw() as u64;
        let at64 = |p: [U<64>; 3], i: i32, j: i32| {
            r64(p[0])
                .wrapping_add(r64(p[1]).wrapping_mul(i as u64))
                .wrapping_add(r64(p[2]).wrapping_mul(j as u64))
        };
        for y in y0..=y1 {
            for x in x0..=x1 {
                if !inside(op, x, y) {
                    continue;
                }
                let at = y as usize * w + x as usize;
                let mut src = colour(op, x - x0, y - y0);
                if let Some((t, d)) = &tex {
                    let (i, j) = (x - x0, y - y0);
                    let uq = at64([op.u0, op.udx, op.udy], i, j);
                    let vq = at64([op.v0, op.vdx, op.vdy], i, j);
                    let q = at64([op.q0, op.qdx, op.qdy], i, j);
                    let (u, v) = crate::tex::texel_uv(uq, vq, q);
                    let n = crate::tex::numerators(op, i, j);
                    let l = crate::tex::lod(n, op.lodk.raw() as u32, q);
                    let ct = crate::tex::sample(d, u, v, l, t.mem);
                    let mode = op.tenv.raw() as u32;
                    let cc = op.tenvc.raw() as u32;
                    src = crate::tex::env(mode, d.class, src, ct, cc);
                }
                if fogged {
                    let (i, j) = (x - x0, y - y0);
                    let f = channel(raw(op.f0), raw(op.fdx), raw(op.fdy), i, j);
                    src = fog(src, f, op.fcol.raw() as u32);
                }
                let aref = op.aref.raw() as u32;
                if atest && !passes(op.afunc.raw() as u32, src >> 24, aref) {
                    continue;
                }
                let z =
                    depth(raw(op.z0), raw(op.zdx), raw(op.zdy), x - x0, y - y0);
                let deep = !on || passes(func, z, zb[at]);
                // The stencil test, then the depth test, and the stencil's
                // operation for how the pixel did in them (issue 998).
                if sten {
                    let s = sb[at];
                    let ok =
                        passes(op.sfunc.raw() as u32, sref & smask, s & smask);
                    let o = match (ok, deep) {
                        (false, _) => op.sfail,
                        (true, false) => op.szfail,
                        (true, true) => op.szpass,
                    };
                    let new = crate::op::stencil::op(o.raw() as u32, s, sref);
                    sb[at] = (s & !swmask) | (new & swmask);
                    if !ok {
                        continue;
                    }
                }
                if !deep {
                    continue;
                }
                let dst = fb[at];
                // A logic operation is in place of the blend, as GL has it.
                let out = match logic {
                    Some(lop) => crate::op::logic(lop, src, dst),
                    None if blending => blend(src, dst, sf, df),
                    None => src,
                };
                fb[at] = masked(out, dst, mask);
                if on && op.zwrite.to_bool() {
                    zb[at] = z;
                }
            }
        }
    }
    fb
}

#[cfg(test)]
mod tests {
    use super::{blend, div255, fog, masked};
    use crate::op::{DST_COLOR, ZERO};
    use crate::op::{ONE, ONE_MINUS_SRC_ALPHA, SRC_ALPHA, SRC_ALPHA_SATURATE};

    /// The divide by 255 without a divider rounds to the nearest over the
    /// whole of what one product and another sum to, and stays at 255 or
    /// more past it, where the blend clamps (issue 993).
    #[test]
    fn the_divide_by_255_rounds_to_the_nearest() {
        for x in 0..=2 * 255 * 255u32 {
            let want = (2 * x + 255) / 510;
            if x <= 255 * 255 {
                assert_eq!(div255(x), want, "{x}");
            } else {
                assert!(div255(x) >= 255, "{x}");
            }
        }
    }

    /// The blend is GL's sum of the two factored colours, a channel at a
    /// time, clamped.
    #[test]
    fn the_blend_is_gls() {
        let (s, d) = (0x80ff_4000, 0xff00_80ff);
        assert_eq!(blend(s, d, ONE, ZERO), s);
        assert_eq!(blend(s, d, ZERO, ONE), d);
        // Half of each, the source's alpha being 0x80.
        let half = blend(s, d, SRC_ALPHA, ONE_MINUS_SRC_ALPHA);
        assert_eq!(half, 0xbf80_607f, "{half:08x}");
        assert_eq!(blend(s, d, ONE, ONE), 0xffff_c0ff, "the sum clamps");
        // Modulate: the source times the destination.
        assert_eq!(blend(s, d, DST_COLOR, ZERO), 0x8000_2000);
        // Saturate takes the source's alpha, at most what the
        // destination leaves, and one for alpha itself.
        assert_eq!(blend(s, d, SRC_ALPHA_SATURATE, ZERO), 0x8000_0000);
        assert_eq!(masked(0x1122_3344, 0x5566_7788, 0b1010), 0x1166_3388);
    }

    /// Fog is GL's `f C + (1 - f) C_f` in red, green and blue, and leaves
    /// the alpha (issue 998).
    #[test]
    fn the_fog_is_gls() {
        let (c, fc) = (0x80ff_4000, 0x20_80ff);
        assert_eq!(fog(c, 255, fc), c, "no fog");
        assert_eq!(fog(c, 0, fc), 0x8020_80ff, "all fog");
        assert_eq!(fog(c, 0x80, fc), 0x8090_607f, "half");
    }
}
