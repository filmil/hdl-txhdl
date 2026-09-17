// SPDX-License-Identifier: Apache-2.0
//! The rasteriser written with loops: the rule the hardware is
//! checked against. It decodes the same instructions and tests the
//! same edge functions, but it says them as a program rather than as
//! a step per cycle, so that the two agreeing means something.
use crate::op::{signed, Insn, Kind};

/// An edge function of the edge from `a` to `b`, at `p`. Positive on
/// one side, negative on the other, zero on the edge.
fn edge(a: (i32, i32), b: (i32, i32), p: (i32, i32)) -> i32 {
    (b.0 - a.0) * (p.1 - a.1) - (b.1 - a.1) * (p.0 - a.0)
}

/// Whether a pixel is in the entry: every pixel of a clear or a
/// rectangle, and of a triangle the pixels at which no edge function
/// is negative.
fn inside(op: &Insn, x: i32, y: i32) -> bool {
    if op.kind != Kind::Tri {
        return true;
    }
    let v = |x, y| (signed(x), signed(y));
    let (a, b, c) = (v(op.ax, op.ay), v(op.bx, op.by), v(op.cx, op.cy));
    let p = (x, y);
    edge(a, b, p) >= 0 && edge(b, c, p) >= 0 && edge(c, a, p) >= 0
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

/// A display list rendered into a framebuffer of `w` by `h` pixels.
pub fn render(ops: &[Insn], w: usize, h: usize) -> Vec<u32> {
    let mut fb = vec![0u32; w * h];
    for op in ops {
        let (x0, y0, x1, y1) = box_of(op, w, h);
        for y in y0..=y1 {
            for x in x0..=x1 {
                if inside(op, x, y) {
                    fb[y as usize * w + x as usize] = op.colour.raw() as u32;
                }
            }
        }
    }
    fb
}
