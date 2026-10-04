// SPDX-License-Identifier: Apache-2.0
//! The rasteriser written with loops: the rule the hardware is
//! checked against. It decodes the same instructions and tests the
//! same edge functions, but it says them as a program rather than as
//! a step per cycle, so that the two agreeing means something.
use crate::op::{signed, Insn, Kind, SUB};
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

/// The colour an entry writes at the pixel `i` right and `j` down of its
/// box's first: its own, or a shaded triangle's three planes there.
fn colour(op: &Insn, i: i32, j: i32) -> u32 {
    if op.kind != Kind::Shaded {
        return op.colour.raw() as u32;
    }
    let p = |a: U<32>, b: U<32>, c: U<32>| {
        channel(a.raw() as u32, b.raw() as u32, c.raw() as u32, i, j)
    };
    let r = p(op.r0, op.rdx, op.rdy);
    let g = p(op.g0, op.gdx, op.gdy);
    let b = p(op.b0, op.bdx, op.bdy);
    (r << 16) | (g << 8) | b
}

/// A display list rendered into a framebuffer of `w` by `h` pixels.
pub fn render(ops: &[Insn], w: usize, h: usize) -> Vec<u32> {
    let mut fb = vec![0u32; w * h];
    for op in ops {
        let (x0, y0, x1, y1) = box_of(op, w, h);
        for y in y0..=y1 {
            for x in x0..=x1 {
                if inside(op, x, y) {
                    fb[y as usize * w + x as usize] =
                        colour(op, x - x0, y - y0);
                }
            }
        }
    }
    fb
}
