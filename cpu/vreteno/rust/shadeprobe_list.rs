// SPDX-License-Identifier: Apache-2.0
//! The picture `shadeprobe` asks Razboj for (issue 989), a Gouraud
//! triangle over a backdrop, as the words of its display list, and the
//! pixel the list leaves at each place, worked out from those same
//! words as Razboj's model works it out. No hardware in it, so a host
//! test holds both to Razboj's encoder and its model, and the program
//! on the board compares every pixel of the screen with it.

/// The words of an entry, `razboj::dl::WORDS`.
pub const WORDS: usize = 16;
/// The entries: the backdrop and the triangle.
pub const ENTRIES: usize = 2;
/// The screen shown, and the words from one row to the next.
pub const W: u32 = 640;
pub const H: u32 = 480;
pub const ROW: u32 = 1024;

/// The backdrop, `0xRRGGBB`, with an alpha of zero as every entry here
/// has, so a pixel is its colour as the scanout shows it.
pub const BACK: u32 = 0x00_00_33;

/// A rectangle: kind 1, both corners included.
const fn rect(colour: u32, x0: u32, y0: u32, x1: u32, y1: u32) -> [u32; WORDS] {
    let mut w = [0u32; WORDS];
    w[0] = 1 | (colour << 2);
    w[1] = x0 | (y0 << 16);
    w[2] = x1 | (y1 << 16);
    w
}

/// The list: the backdrop over the screen, then the triangle with its
/// corners at 320, 40 in red, 580, 440 in green and 60, 440 in blue, as
/// `razboj::op::Op::Gouraud` encodes it on a 640 by 480 screen, which
/// the host test checks word for word.
pub const LIST: [[u32; WORDS]; ENTRIES] = [
    rect(BACK, 0, 0, 639, 479),
    [
        0x03fc_0003, // kind 3, the first colour
        0x0028_003c, // the box: x0, y0
        0x01b8_0244, // x1, y1
        0x0280_1400, // a, in sixteenths of a pixel
        0x1b80_2440, // b
        0x1b80_03c0, // c
        0x00ff_2e66, // red at the first pixel
        0x0000_0000, // red a pixel across
        0xffff_5ccd, // red a row down
        0xff81_6792, // green at the first pixel
        0x0000_7d8a, // green a pixel across
        0x0000_519a, // green a row down
        0x007f_ea08, // blue at the first pixel
        0xffff_8276, // blue a pixel across
        0x0000_519a, // blue a row down
        0x0000_0000, // alpha
    ],
];

/// A vertex word's two coordinates, in sixteenths of a pixel.
fn vertex(w: u32) -> (i64, i64) {
    ((w as u16 as i16) as i64, ((w >> 16) as u16 as i16) as i64)
}

/// The edge function of the edge from `a` to `b` at `p`.
fn edge(a: (i64, i64), b: (i64, i64), p: (i64, i64)) -> i64 {
    (b.0 - a.0) * (p.1 - a.1) - (b.1 - a.1) * (p.0 - a.0)
}

/// Whether a centre on the edge from `a` to `b` is inside: a top or a
/// left edge, as the rasteriser has it.
fn top_left(a: (i64, i64), b: (i64, i64)) -> bool {
    let (ddx, ddy) = (-(b.1 - a.1), b.0 - a.0);
    ddx > 0 || (ddx == 0 && ddy > 0)
}

/// One channel: its start and steps, `i` across and `j` down, wrapped
/// in thirty-two bits and clamped to a byte.
fn channel(start: u32, dx: u32, dy: u32, i: u32, j: u32) -> u32 {
    let v = start
        .wrapping_add(dx.wrapping_mul(i))
        .wrapping_add(dy.wrapping_mul(j));
    if v & 0x8000_0000 != 0 {
        0
    } else if v >> 24 != 0 {
        255
    } else {
        (v >> 16) & 0xff
    }
}

/// The word the list leaves at the pixel `x`, `y` of the screen: the
/// triangle's shade where the pixel's centre is inside it, and the
/// backdrop elsewhere.
pub fn expect(x: u32, y: u32) -> u32 {
    let t = &LIST[1];
    let (x0, y0) = (t[1] & 0x3ff, (t[1] >> 16) & 0x3ff);
    let (x1, y1) = (t[2] & 0x3ff, (t[2] >> 16) & 0x3ff);
    if x < x0 || x > x1 || y < y0 || y > y1 {
        return BACK;
    }
    let (a, b, c) = (vertex(t[3]), vertex(t[4]), vertex(t[5]));
    let p = (x as i64 * 16 + 8, y as i64 * 16 + 8);
    let holds = |a, b| {
        let e = edge(a, b, p);
        e > 0 || (e == 0 && top_left(a, b))
    };
    if !(holds(a, b) && holds(b, c) && holds(c, a)) {
        return BACK;
    }
    let (i, j) = (x - x0, y - y0);
    let r = channel(t[6], t[7], t[8], i, j);
    let g = channel(t[9], t[10], t[11], i, j);
    let bl = channel(t[12], t[13], t[14], i, j);
    ((t[15] & 0xff) << 24) | (r << 16) | (g << 8) | bl
}
