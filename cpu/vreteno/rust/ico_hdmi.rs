// SPDX-License-Identifier: Apache-2.0
//! A turning icosahedron on the board's HDMI output, drawn by the core
//! into the video peripheral's framebuffer, with the TxHDL logo in a
//! corner.
//!
//! The peripheral is the third slot of the page at `0x3000`, at
//! `0x3200`, which on the flagship reaches the video peripheral on the
//! pixel clock. Three words: the status, the cursor, and the pixel.
//! Writing the pixel word puts a colour at the cursor and moves the
//! cursor to the next column, and from the last column to the first of
//! the next row, so a run of pixels along a scanline is one write of
//! the cursor and then one write per pixel.
//!
//! The framebuffer is 160 by 120, and each of its pixels covers four by
//! four of the screen's 640 by 480. A colour is twelve bits, four each
//! of red, green and blue.
//!
//! This program is linked for the memory at `0x4000_0000` and sent down
//! the serial port by the loader, so the picture changes without
//! Vivado. That is what the flagship is for.
//!
//! ## The solid
//!
//! An icosahedron's twelve vertices are the corners of three golden
//! rectangles: `(0, +-1, +-phi)` and its two cyclic rotations. The
//! twenty faces are not written down here. They are worked out at
//! startup, by taking every triple of vertices whose three edges are
//! the shortest distance in the solid, which is what a face is, and
//! then turning each triple so that its normal points away from the
//! centre. A table typed from a book is a table that can be wrong in a
//! way nothing catches; this cannot be.
//!
//! ## The arithmetic
//!
//! Fixed point throughout, ten fractional bits, in `i32`. The core has
//! no floating point. A sine is a quarter-wave table of 65 entries,
//! read forwards and backwards and negated, which is exact at the ends
//! and needs no interpolation at this size.
//!
//! ## What is drawn
//!
//! The face normals are unit vectors worked out once and turned with
//! the solid, so one number does two jobs: a face is visible when its
//! turned normal points towards the viewer, and how brightly it is lit
//! is how directly it does. The light is at the viewer's eye. The
//! solid is convex, so a normal pointing towards the viewer is the
//! whole of hidden surface removal, and no depth buffer is needed.
//!
//! ## One write per pixel
//!
//! There is one framebuffer and the raster reads it while this writes,
//! so anything written twice in a frame is seen in between. Clearing
//! the screen and then drawing on it does exactly that: for the
//! milliseconds between the two, most of the screen is background, and
//! what a viewer sees is a flicker rather than a picture.
//!
//! So nothing is cleared. A row is composed in a buffer here, the
//! background and the solid and the logo together, and then written to
//! the framebuffer once, left to right. Every pixel is written exactly
//! once per frame and goes straight from what it was to what it should
//! be. A moving edge can still tear, because the raster and the writer
//! cross somewhere, but nothing flashes.
//!
//! Composing a row needs the faces that cover it, and the solid is
//! convex, so the faces that survive the normal test tile the
//! silhouette without overlapping: a row is a set of disjoint spans on
//! a background, which is why one pass can write it.
//!
//! The program waits for the vertical blanking before each frame, so
//! the crossing point stays in one place rather than walking.
#![no_std]
#![no_main]

use core::panic::PanicInfo;
use core::ptr::{read_volatile, write_volatile};
use vreteno_regs::uart;

/// The serial port, as `hello.rs` has it.
const UART: *mut u32 = 0x3000 as *mut u32;
/// The video peripheral, the third slot of the same page.
const VIDEO: *mut u32 = 0x3200 as *mut u32;
/// Its words: the status reads, the cursor reads and writes, and the
/// pixel writes.
const STATUS: usize = 0;
const CURSOR: usize = 1;
const PIXEL: usize = 2;
/// Bit 0 of the status is high while the raster is in the vertical
/// blanking, which is the moment to start a frame.
const BLANKING: u32 = 1;

/// The framebuffer, in its own pixels.
const W: i32 = 160;
const H: i32 = 120;

/// The fractional bits of every fixed point number here.
const SHIFT: i32 = 10;
const ONE: i32 = 1 << SHIFT;

/// The viewer's distance, and the projection's scale. The solid's
/// radius is about 1.9, so the eye is eight radii away: close enough
/// that the near faces are visibly larger than the far ones, and far
/// enough that they do not balloon. At three radii, which is where
/// this started, the nearest face grew to nearly the whole silhouette
/// and read as a back face painted over the front.
const D: i32 = 16 * ONE;
const PROJ: i32 = 330;

/// The colour of the solid at full light, four bits a channel: the tan
/// of the hat in the logo.
const LIT_R: i32 = 13;
const LIT_G: i32 = 9;
const LIT_B: i32 = 5;
/// What is behind it: near black, with a little blue, as the logo's own
/// background is.
const BACKDROP: u16 = 0x001;

/// A quarter wave of a sine, in the fixed point above: 65 entries, so
/// that both ends land exactly on zero and on one.
const QUARTER: usize = 64;
static SINE: [i16; QUARTER + 1] = [
    0, 25, 50, 75, 100, 125, 150, 175, 200, 224, 249, 273, 297, 321, 345, 369,
    392, 415, 438, 460, 483, 505, 526, 548, 569, 590, 610, 630, 650, 669, 688,
    706, 724, 742, 759, 775, 792, 807, 822, 837, 851, 865, 878, 891, 903, 915,
    926, 936, 946, 955, 964, 972, 980, 987, 993, 999, 1004, 1009, 1013, 1016,
    1019, 1021, 1023, 1024, 1024,
];

/// Angles run from 0 to 256 for a full turn, so a quarter is 64 and the
/// table above is indexed directly.
fn sin(a: i32) -> i32 {
    let a = a & 255;
    match a >> 6 {
        0 => SINE[a as usize] as i32,
        1 => SINE[(128 - a) as usize] as i32,
        2 => -(SINE[(a - 128) as usize] as i32),
        _ => -(SINE[(256 - a) as usize] as i32),
    }
}

fn cos(a: i32) -> i32 {
    sin(a + 64)
}

/// A multiply that keeps the fixed point where it was.
fn mul(a: i32, b: i32) -> i32 {
    ((a as i64 * b as i64) >> SHIFT) as i32
}

/// The square root of a fixed point number, by Newton's method from a
/// rough start. Used once per face at startup and never again.
fn sqrt(v: i32) -> i32 {
    if v <= 0 {
        return 0;
    }
    let mut x = v;
    let mut i = 0;
    while i < 24 {
        let d = (v << SHIFT) / x;
        let next = (x + d) >> 1;
        if next == x {
            return x;
        }
        x = next;
        i += 1;
    }
    x
}

// ---------------------------------------------------------------
// The serial port, for the line this says when it starts.
// ---------------------------------------------------------------

fn put(byte: u8) {
    unsafe {
        while read_volatile(UART.add(uart::STATUS / 4))
            & uart::STATUS_BUSY_MASK
            != 0
        {}
        write_volatile(UART.add(uart::TX / 4), byte as u32);
    }
}

fn say(text: &str) {
    for byte in text.as_bytes() {
        put(*byte);
    }
}

// ---------------------------------------------------------------
// The peripheral.
// ---------------------------------------------------------------

/// Put the cursor at a column and a row.
fn cursor(x: i32, y: i32) {
    unsafe {
        write_volatile(
            VIDEO.add(CURSOR),
            ((y as u32) << 8) | (x as u32 & 0xff),
        );
    }
}

/// Write one pixel at the cursor, and move on.
fn pixel(colour: u32) {
    unsafe {
        write_volatile(VIDEO.add(PIXEL), colour);
    }
}

/// Wait for the raster to reach the vertical blanking, and then for it
/// to leave, so that a frame starts at a known place rather than
/// wherever the last one finished.
fn wait_blanking() {
    unsafe {
        while read_volatile(VIDEO.add(STATUS)) & BLANKING != 0 {}
        while read_volatile(VIDEO.add(STATUS)) & BLANKING == 0 {}
    }
}

/// Write a composed row to the framebuffer, left to right. One cursor
/// write and `W` pixel writes, and every pixel of the row is written
/// exactly once in the frame.
fn put_row(row: &[u16; W as usize], y: i32) {
    cursor(0, y);
    let mut x = 0;
    while x < W as usize {
        pixel(row[x] as u32);
        x += 1;
    }
}

/// Paint a run into a composed row. The ends are in sixteenths of a
/// sixteenth of a pixel, as `along` gives them, and the run is half
/// open: it starts at the first pixel centre at or after the left end
/// and stops before the first at or after the right end. Two faces
/// that share an edge compute the same edge, so between them they
/// paint each pixel of it exactly once, with no crack and no overlap.
fn span(
    row: &mut [u16; W as usize],
    left: i32,
    right: i32,
    shades: &[u16; 16],
    level: u8,
    y: i32,
) {
    let mut x = (left + 0xffff) >> 16;
    let end = (right + 0xffff) >> 16;
    if x < 0 {
        x = 0;
    }
    let end = if end > W { W } else { end };
    while x < end {
        row[x as usize] = shade(shades, level, x, y);
        x += 1;
    }
}

// ---------------------------------------------------------------
// The solid.
// ---------------------------------------------------------------

const VERTS: usize = 12;
const FACES: usize = 20;

/// The twelve vertices, in the fixed point above: `phi` is 1.618, and
/// `1` and `phi` scaled by `ONE`.
const PHI: i32 = 1657;
static BODY: [[i32; 3]; VERTS] = [
    [0, ONE, PHI],
    [0, ONE, -PHI],
    [0, -ONE, PHI],
    [0, -ONE, -PHI],
    [ONE, PHI, 0],
    [ONE, -PHI, 0],
    [-ONE, PHI, 0],
    [-ONE, -PHI, 0],
    [PHI, 0, ONE],
    [PHI, 0, -ONE],
    [-PHI, 0, ONE],
    [-PHI, 0, -ONE],
];

/// The square of the edge, which for these coordinates is exactly two,
/// and the slack allowed when comparing against it.
const EDGE2: i64 = (2 * ONE as i64) * (2 * ONE as i64);
const SLACK: i64 = EDGE2 / 16;

fn dist2(a: usize, b: usize) -> i64 {
    let mut total: i64 = 0;
    let mut i = 0;
    while i < 3 {
        let d = (BODY[a][i] - BODY[b][i]) as i64;
        total += d * d;
        i += 1;
    }
    total
}

fn is_edge(a: usize, b: usize) -> bool {
    let d = dist2(a, b);
    d > EDGE2 - SLACK && d < EDGE2 + SLACK
}

/// The cross product of two vectors, in the fixed point above.
fn cross(u: [i32; 3], v: [i32; 3]) -> [i32; 3] {
    [
        mul(u[1], v[2]) - mul(u[2], v[1]),
        mul(u[2], v[0]) - mul(u[0], v[2]),
        mul(u[0], v[1]) - mul(u[1], v[0]),
    ]
}

fn minus(a: [i32; 3], b: [i32; 3]) -> [i32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn dot(a: [i32; 3], b: [i32; 3]) -> i32 {
    mul(a[0], b[0]) + mul(a[1], b[1]) + mul(a[2], b[2])
}

/// Work out the faces: every triple of vertices that are pairwise an
/// edge apart, wound so that the normal points away from the centre,
/// with that normal made a unit vector. Returns how many were found,
/// which is twenty for an icosahedron and is checked by the caller.
fn build(
    face: &mut [[usize; 3]; FACES],
    normal: &mut [[i32; 3]; FACES],
) -> usize {
    let mut found = 0;
    let mut a = 0;
    while a < VERTS {
        let mut b = a + 1;
        while b < VERTS {
            if is_edge(a, b) {
                let mut c = b + 1;
                while c < VERTS {
                    if is_edge(a, c) && is_edge(b, c) && found < FACES {
                        let (mut i, mut j, k) = (a, b, c);
                        let mut n = cross(
                            minus(BODY[j], BODY[i]),
                            minus(BODY[k], BODY[i]),
                        );
                        // The centre of the solid is the origin, so a
                        // normal that points outwards agrees with any
                        // of the triple's own positions.
                        if dot(n, BODY[i]) < 0 {
                            let t = i;
                            i = j;
                            j = t;
                            n = cross(
                                minus(BODY[j], BODY[i]),
                                minus(BODY[k], BODY[i]),
                            );
                        }
                        let len = sqrt(dot(n, n));
                        if len > 0 {
                            face[found] = [i, j, k];
                            normal[found] = [
                                (n[0] << SHIFT) / len,
                                (n[1] << SHIFT) / len,
                                (n[2] << SHIFT) / len,
                            ];
                            found += 1;
                        }
                    }
                    c += 1;
                }
            }
            b += 1;
        }
        a += 1;
    }
    found
}

/// Turn a point about the Y axis and then the X axis.
fn turn(p: [i32; 3], ay: i32, ax: i32) -> [i32; 3] {
    let (sy, cy) = (sin(ay), cos(ay));
    let x = mul(p[0], cy) + mul(p[2], sy);
    let z = mul(p[2], cy) - mul(p[0], sy);
    let (sx, cx) = (sin(ax), cos(ax));
    let y = mul(p[1], cx) - mul(z, sx);
    let z = mul(z, cx) + mul(p[1], sx);
    [x, y, z]
}

/// Project a turned point onto the framebuffer.
fn project(p: [i32; 3]) -> [i32; 2] {
    let denom = D - p[2];
    let scale = PROJ;
    [
        W / 2 + (p[0] * scale) / denom,
        H / 2 - (p[1] * scale) / denom,
    ]
}

/// A face ready to draw: its corners sorted by row, the rows it
/// covers, and the colour its light gives it.
#[derive(Clone, Copy)]
struct Ready {
    p: [[i32; 2]; 3],
    y0: i32,
    y1: i32,
    level: u8,
}

impl Ready {
    const NONE: Ready = Ready {
        p: [[0, 0], [0, 0], [0, 0]],
        y0: 1,
        y1: 0,
        level: 0,
    };

    /// Sort the corners by row, which is three compares, and note the
    /// rows the triangle covers.
    fn new(mut p: [[i32; 2]; 3], level: u8) -> Ready {
        let mut i = 0;
        while i < 2 {
            let mut j = 0;
            while j < 2 - i {
                if p[j][1] > p[j + 1][1] {
                    let t = p[j];
                    p[j] = p[j + 1];
                    p[j + 1] = t;
                }
                j += 1;
            }
            i += 1;
        }
        Ready {
            p,
            y0: p[0][1],
            y1: p[2][1],
            level,
        }
    }

    /// Paint this face's part of one row, if it covers it.
    fn row(&self, shades: &[u16; 16], row: &mut [u16; W as usize], y: i32) {
        if y < self.y0 || y > self.y1 {
            return;
        }
        let (top, mid, bot) = (self.p[0], self.p[1], self.p[2]);
        if bot[1] == top[1] {
            span(
                row,
                min3(top[0], mid[0], bot[0]) << 16,
                max3(top[0], mid[0], bot[0]) << 16,
                shades,
                self.level,
                y,
            );
            return;
        }
        // The long edge, from the top corner to the bottom one, and
        // whichever short edge this row crosses.
        let xa = along(top, bot, y);
        let xb = if y < mid[1] {
            along(top, mid, y)
        } else {
            along(mid, bot, y)
        };
        if xa <= xb {
            span(row, xa, xb, shades, self.level, y);
        } else {
            span(row, xb, xa, shades, self.level, y);
        }
    }
}

/// Where an edge is at a row, in sixteenths of a sixteenth of a pixel,
/// so that the fill can round it the same way from either side. An
/// edge with no height is at its own start, which is what the caller
/// wants for a flat top or bottom.
fn along(a: [i32; 2], b: [i32; 2], y: i32) -> i32 {
    if b[1] == a[1] {
        return a[0] << 16;
    }
    let rise = (b[0] - a[0]) as i64;
    let run = (b[1] - a[1]) as i64;
    ((a[0] as i64) * 65536 + (rise * 65536 * (y - a[1]) as i64) / run) as i32
}

fn min3(a: i32, b: i32, c: i32) -> i32 {
    let m = if a < b { a } else { b };
    if m < c {
        m
    } else {
        c
    }
}

fn max3(a: i32, b: i32, c: i32) -> i32 {
    let m = if a > b { a } else { b };
    if m > c {
        m
    } else {
        c
    }
}

/// Whether a pixel between two steps of the ramp is dithered: a two
/// by two pattern of the two steps rather than the nearer one alone,
/// which quadruples the levels the eye sees at the cost of a fine
/// checker on the face. A framebuffer pixel is four screen pixels
/// square, so the checker is eight screen pixels, which is visible up
/// close and reads as a level from a chair. Off, the nearer step alone.
const DITHER: bool = true;

/// How much of a face's light is there whatever way it faces. Below
/// this the ramp's steps are far apart, a step of the darkest few is
/// half the brightness of the last, and a face crossing one snaps.
/// With the floor, a face uses the upper steps, where neighbours
/// differ by a tenth.
const AMBIENT: i32 = ONE * 2 / 5;

/// The sixteen shades of the solid, from dark to the colour at full
/// light, worked out once at startup. The step's brightness follows a
/// square root rather than a line, which spaces the steps evenly to
/// the eye rather than to a meter; each is the full colour scaled by
/// that and rounded to the nearest sixteenth, so every step is the
/// same tan a little brighter and nothing else.
///
/// This replaced a formula that scaled each channel on its own and
/// rounded each on its own. With four bits a channel the three
/// channels then crossed their steps at different moments, and a face
/// turning smoothly changed hue rather than brightness: tan to pink to
/// grey to green, as the video showed. With one ramp the light picks a
/// step and the step is a colour, and a face only ever moves along it.
fn ramp(shades: &mut [u16; 16]) {
    let mut k = 0;
    while k < 16 {
        // The square root of k / 15, in the fixed point.
        let bright = sqrt((k as i32 * ONE) / 15);
        let c =
            |full: i32| -> u16 { ((full * bright + ONE / 2) >> SHIFT) as u16 };
        shades[k] = c(LIT_R) << 8 | c(LIT_G) << 4 | c(LIT_B);
        k += 1;
    }
}

/// The level of a face lit by `light`, which runs from zero to `ONE`:
/// the ramp's step in the top four bits and the quarter of the way to
/// the next step in the bottom two, for the dither.
fn level(light: i32) -> u8 {
    let lit = AMBIENT + ((ONE - AMBIENT) * light >> SHIFT);
    let mut k = (lit * 60 + ONE / 2) >> SHIFT;
    if k > 60 {
        k = 60;
    }
    k as u8
}

/// The colour of a pixel at a level: the step, or with the dither the
/// step or the next by where the pixel is in the two by two pattern.
fn shade(shades: &[u16; 16], level: u8, x: i32, y: i32) -> u16 {
    let step = (level >> 2) as usize;
    if !DITHER || step >= 15 {
        return shades[step];
    }
    let frac = level & 3;
    // The thresholds of a two by two ordered dither.
    let threshold = match ((y & 1) << 1) | (x & 1) {
        0 => 0,
        1 => 2,
        2 => 3,
        _ => 1,
    };
    if frac > threshold {
        shades[step + 1]
    } else {
        shades[step]
    }
}

/// Whether the sixteen steps of the ramp are shown along the top of
/// the screen, each an eight pixel square, so that a person at the
/// monitor can tell a step that looks wrong from a face that crossed
/// one. A check rather than a feature; off for a picture.
const SWATCHES: bool = false;

/// Compose the swatches into a row, if they reach it.
fn swatch_row(shades: &[u16; 16], row: &mut [u16; W as usize], y: i32) {
    if !SWATCHES || y < 2 || y >= 10 {
        return;
    }
    let mut k = 0;
    while k < 16 {
        let x0 = 16 + k as i32 * 8;
        let mut x = x0;
        while x < x0 + 8 {
            row[x as usize] = shades[k];
            x += 1;
        }
        k += 1;
    }
}

/// Where the logo sits: the bottom right corner, two pixels in from
/// each edge.
const LOGO_X: i32 = W - txhdl_logo::W as i32 - 2;
const LOGO_Y: i32 = H - txhdl_logo::H as i32 - 2;

/// Compose the logo into a row, if the logo reaches it. A transparent
/// pixel is left as whatever the solid or the background put there, so
/// the picture shows through around the hat.
fn logo_row(row: &mut [u16; W as usize], y: i32) {
    let r = y - LOGO_Y;
    if r < 0 || r >= txhdl_logo::H as i32 {
        return;
    }
    let mut col = 0;
    while col < txhdl_logo::W as i32 {
        let word =
            txhdl_logo::PIXELS[(r * txhdl_logo::W as i32 + col) as usize];
        let x = LOGO_X + col;
        if word != txhdl_logo::TRANSPARENT && x >= 0 && x < W {
            row[x as usize] = word;
        }
        col += 1;
    }
}

/// The entry point, where the core's program counter starts.
#[no_mangle]
#[link_section = ".text.init"]
pub unsafe extern "C" fn _start() -> ! {
    core::arch::asm!(
        "la sp, __stack_top",
        "la t0, __bss_start",
        "la t1, __bss_end",
        "1:",
        "beq t0, t1, 2f",
        "sw zero, 0(t0)",
        "addi t0, t0, 4",
        "j 1b",
        "2:",
        "j {main}",
        main = sym main,
        options(noreturn)
    )
}

#[no_mangle]
extern "C" fn main() -> ! {
    let mut face = [[0usize; 3]; FACES];
    let mut normal = [[0i32; 3]; FACES];
    let found = build(&mut face, &mut normal);

    say("ico ");
    put(b'0' + (found / 10) as u8);
    put(b'0' + (found % 10) as u8);
    say(" faces\n");
    let mut ay: i32 = 0;
    let mut ax: i32 = 0;
    let mut shades = [0u16; 16];
    ramp(&mut shades);
    let mut ready = [Ready::NONE; FACES];
    let mut row = [0u16; W as usize];
    loop {
        wait_blanking();

        // The vertices, turned once and projected once, rather than
        // once per face that uses them.
        let mut screen = [[0i32; 2]; VERTS];
        let mut v = 0;
        while v < VERTS {
            screen[v] = project(turn(BODY[v], ay, ax));
            v += 1;
        }

        // The faces that can be seen, with the light they catch. The
        // normal turns with the solid, so it points towards the viewer
        // exactly when the face is one that can be seen, and how far
        // it leans says how brightly it is lit.
        let mut shown = 0;
        let mut f = 0;
        while f < found {
            let n = turn(normal[f], ay, ax);
            if n[2] > 0 {
                ready[shown] = Ready::new(
                    [
                        screen[face[f][0]],
                        screen[face[f][1]],
                        screen[face[f][2]],
                    ],
                    level(n[2]),
                );
                shown += 1;
            }
            f += 1;
        }

        // One pass down the screen. Each row is composed whole and
        // then written once, so no pixel is ever seen half drawn.
        let mut y = 0;
        while y < H {
            let mut x = 0;
            while x < W as usize {
                row[x] = BACKDROP;
                x += 1;
            }
            let mut i = 0;
            while i < shown {
                ready[i].row(&shades, &mut row, y);
                i += 1;
            }
            logo_row(&mut row, y);
            swatch_row(&shades, &mut row, y);
            put_row(&row, y);
            y += 1;
        }

        ay = (ay + 2) & 255;
        ax = (ax + 1) & 255;
    }
}

#[panic_handler]
fn panicked(_: &PanicInfo) -> ! {
    loop {}
}
