// SPDX-License-Identifier: Apache-2.0
//! The picture `razprobe` asks Razboj for (issue 1169), as the words of
//! its display list, and the pixels the program reads back to check
//! it. No hardware in it, so a host test renders the same list through
//! Razboj's model and holds the checks to it.

/// The words of an entry, `razboj::dl::WORDS`.
pub const WORDS: usize = 16;
/// The entries: the backdrop and three shapes.
pub const ENTRIES: usize = 4;

/// The colours, `0xRRGGBB`, and the alpha every entry writes, which is
/// zero, so a pixel is its colour as the scanout shows it.
pub const BACK: u32 = 0x00_00_33;
pub const RED: u32 = 0xff_20_20;
pub const GREEN: u32 = 0x20_ff_20;
pub const SKY: u32 = 0x80_c0_ff;

/// A rectangle: kind 1, both corners included.
const fn rect(colour: u32, x0: u32, y0: u32, x1: u32, y1: u32) -> [u32; WORDS] {
    let mut w = [0u32; WORDS];
    w[0] = 1 | (colour << 2);
    w[1] = x0 | (y0 << 16);
    w[2] = x1 | (y1 << 16);
    w
}

const fn min3(a: u32, b: u32, c: u32) -> u32 {
    let m = if a < b { a } else { b };
    if m < c {
        m
    } else {
        c
    }
}

const fn max3(a: u32, b: u32, c: u32) -> u32 {
    let m = if a > b { a } else { b };
    if m > c {
        m
    } else {
        c
    }
}

/// A flat triangle: kind 2, its box, and its corners in sixteenths of
/// a pixel, given wound so that inside is where no edge function is
/// negative: clockwise on the screen, whose rows run down.
const fn tri(
    colour: u32,
    p: [u32; 2],
    q: [u32; 2],
    r: [u32; 2],
) -> [u32; WORDS] {
    let mut w = [0u32; WORDS];
    w[0] = 2 | (colour << 2);
    w[1] = min3(p[0], q[0], r[0]) | (min3(p[1], q[1], r[1]) << 16);
    w[2] = max3(p[0], q[0], r[0]) | (max3(p[1], q[1], r[1]) << 16);
    w[3] = (p[0] * 16) | ((p[1] * 16) << 16);
    w[4] = (q[0] * 16) | ((q[1] * 16) << 16);
    w[5] = (r[0] * 16) | ((r[1] * 16) << 16);
    w
}

/// The list: the backdrop over the 640 by 480 that is shown, a red
/// rectangle at the top left, a green triangle pointing down at the
/// top right, and a sky blue one pointing up at the bottom middle.
pub const LIST: [[u32; WORDS]; ENTRIES] = [
    rect(BACK, 0, 0, 639, 479),
    rect(RED, 40, 40, 239, 199),
    tri(GREEN, [400, 40], [600, 40], [500, 200]),
    tri(SKY, [320, 260], [420, 440], [220, 440]),
];

/// The pixels the program reads back, and what each must hold.
pub const CHECKS: [(u32, u32, u32); 6] = [
    (140, 120, RED),
    (500, 80, GREEN),
    (320, 400, SKY),
    (320, 240, BACK),
    (620, 460, BACK),
    (10, 10, BACK),
];
