// SPDX-License-Identifier: Apache-2.0
//! The rasteriser written with loops: the rule the hardware is
//! checked against. It walks the same boxes and tests the same edge
//! functions, but it says them as a program rather than as a step per
//! cycle, so that the two agreeing means something.
use crate::op::Op;

/// An edge function of the edge from `a` to `b`, at `p`. Positive on
/// one side, negative on the other, zero on the edge.
fn edge(a: (i32, i32), b: (i32, i32), p: (i32, i32)) -> i32 {
    (b.0 - a.0) * (p.1 - a.1) - (b.1 - a.1) * (p.0 - a.0)
}

/// Whether a pixel is in the entry: every pixel of a box, and of a
/// triangle the pixels at which no edge function is negative.
fn inside(op: &Op, x: i32, y: i32) -> bool {
    if op.tri.raw() == 0 {
        return true;
    }
    let v = |x: txhdl::types::U<12>, y: txhdl::types::U<12>| {
        (crate::op::signed(x), crate::op::signed(y))
    };
    let (a, b, c) = (v(op.ax, op.ay), v(op.bx, op.by), v(op.cx, op.cy));
    let p = (x, y);
    edge(a, b, p) >= 0 && edge(b, c, p) >= 0 && edge(c, a, p) >= 0
}

/// A display list rendered into a framebuffer of `w` by `h` pixels.
pub fn render(ops: &[Op], w: usize, h: usize) -> Vec<u32> {
    let mut fb = vec![0u32; w * h];
    for op in ops {
        let x0 = op.x0.raw() as i32;
        let y0 = op.y0.raw() as i32;
        let x1 = op.x1.raw() as i32;
        let y1 = op.y1.raw() as i32;
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
