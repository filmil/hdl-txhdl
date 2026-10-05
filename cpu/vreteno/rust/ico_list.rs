// SPDX-License-Identifier: Apache-2.0
//! The turning icosahedron as Razboj's display lists (issue 986): the
//! geometry `ico_hdmi.rs` worked out to draw on the core, now written as
//! one list a frame for the rasteriser to fill.
//!
//! This file is the part with no hardware in it, so that the program
//! includes it and a test on the host renders the same lists through
//! Razboj's model. It needs no standard library and allocates nothing.
//!
//! ## Two frames in one framebuffer
//!
//! The rasteriser's framebuffer is a constant of its type: rows of 1024
//! words from `0x4200_0000`, and the scanout shows 640 by 480 of it.
//! So the second frame is not a second base. It is rows 512 to 991 of
//! the same framebuffer, at `0x4220_0000`: every row of a frame drawn
//! there is 512 more, and the scanout's base moves between the two.
//!
//! ## What a frame's list holds
//!
//! First a rectangle of the backdrop over the box the solid filled the
//! last time this frame was drawn, which is all of it that can have
//! changed. Then a triangle a face, for every face that faces the
//! camera. The solid is convex, so those faces never overlap and their
//! order does not matter. The rasteriser fills a pixel when its centre
//! is inside, with ties to the top and left edges, so two faces that
//! share an edge fill each pixel of it once.

/// The screen, and the rows from one frame to the other.
pub const W: i32 = 640;
pub const H: i32 = 480;
pub const SECOND: i32 = 512;

/// The words of an entry, `razboj::dl::WORDS`.
pub const WORDS: usize = 16;
/// The entries a frame's list can hold: a clear and twenty faces.
pub const MOST: usize = 1 + FACES;

/// The fractional bits of every fixed point number here.
const SHIFT: i32 = 10;
const ONE: i32 = 1 << SHIFT;

/// The viewer's distance, and the projection's scale in pixels.
const D: i32 = 16 * ONE;
const PROJ: i32 = 1200;

/// The furthest a vertex is from the centre, a little over the
/// solid's circumradius of 1.902, and so the furthest from the
/// screen's middle a vertex can land: at that distance from the
/// centre and as near the viewer as it can be.
const RADIUS: i32 = 1958;
pub const REACH: i32 = RADIUS * PROJ / (D - RADIUS) + 1;

/// The colour of the solid at full light, and of what is behind it.
const LIT: u32 = 0xdd_99_55;
pub const BACKDROP: u32 = 0x00_00_11;
/// How much of a face's light is there whatever way it faces.
const AMBIENT: i32 = ONE * 2 / 5;

/// A quarter wave of a sine, in the fixed point above.
const QUARTER: usize = 64;
static SINE: [i16; QUARTER + 1] = [
    0, 25, 50, 75, 100, 125, 150, 175, 200, 224, 249, 273, 297, 321, 345, 369,
    392, 415, 438, 460, 483, 505, 526, 548, 569, 590, 610, 630, 650, 669, 688,
    706, 724, 742, 759, 775, 792, 807, 822, 837, 851, 865, 878, 891, 903, 915,
    926, 936, 946, 955, 964, 972, 980, 987, 993, 999, 1004, 1009, 1013, 1016,
    1019, 1021, 1023, 1024, 1024,
];

/// Angles run from 0 to 256 for a full turn.
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

/// The square root of a fixed point number, by Newton's method.
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

pub const VERTS: usize = 12;
pub const FACES: usize = 20;

/// The twelve vertices: the corners of three golden rectangles.
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

/// The square of the edge, exactly two for these coordinates, and the
/// slack allowed when comparing against it.
const EDGE2: i64 = (2 * ONE as i64) * (2 * ONE as i64);
const SLACK: i64 = EDGE2 / 16;

fn is_edge(a: usize, b: usize) -> bool {
    let mut d: i64 = 0;
    let mut i = 0;
    while i < 3 {
        let e = (BODY[a][i] - BODY[b][i]) as i64;
        d += e * e;
        i += 1;
    }
    d > EDGE2 - SLACK && d < EDGE2 + SLACK
}

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

/// The solid's faces, worked out rather than typed: every triple of
/// vertices pairwise an edge apart, wound so that the normal points
/// away from the centre, with the normal as a unit vector.
pub struct Solid {
    pub face: [[usize; 3]; FACES],
    pub normal: [[i32; 3]; FACES],
    /// How many were found: twenty for an icosahedron.
    pub found: usize,
}

impl Solid {
    pub fn new() -> Solid {
        let mut s = Solid {
            face: [[0; 3]; FACES],
            normal: [[0; 3]; FACES],
            found: 0,
        };
        let mut a = 0;
        while a < VERTS {
            let mut b = a + 1;
            while b < VERTS {
                let mut c = b + 1;
                while c < VERTS {
                    if is_edge(a, b)
                        && is_edge(a, c)
                        && is_edge(b, c)
                        && s.found < FACES
                    {
                        s.add(a, b, c);
                    }
                    c += 1;
                }
                b += 1;
            }
            a += 1;
        }
        s
    }

    fn add(&mut self, a: usize, b: usize, c: usize) {
        let (mut i, mut j, k) = (a, b, c);
        let mut n = cross(minus(BODY[j], BODY[i]), minus(BODY[k], BODY[i]));
        // The centre is the origin, so an outward normal agrees with
        // the face's own position.
        if dot(n, BODY[i]) < 0 {
            core::mem::swap(&mut i, &mut j);
            n = cross(minus(BODY[j], BODY[i]), minus(BODY[k], BODY[i]));
        }
        let len = sqrt(dot(n, n));
        if len > 0 {
            self.face[self.found] = [i, j, k];
            self.normal[self.found] = [
                (n[0] << SHIFT) / len,
                (n[1] << SHIFT) / len,
                (n[2] << SHIFT) / len,
            ];
            self.found += 1;
        }
    }
}

impl Default for Solid {
    fn default() -> Self {
        Solid::new()
    }
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

/// A turned point on the screen, in sixteenths of a pixel, the units
/// of a vertex in the list.
fn project(p: [i32; 3]) -> [i32; 2] {
    let denom = D - p[2];
    [
        (W / 2) * 16 + (p[0] * PROJ * 16) / denom,
        (H / 2) * 16 - (p[1] * PROJ * 16) / denom,
    ]
}

/// The colour of a face lit by `light`, from zero to `ONE`: the full
/// colour scaled by the square root of how lit it is, which spaces the
/// shades evenly to the eye rather than to a meter.
fn shade(light: i32) -> u32 {
    let light = light.clamp(0, ONE);
    let lit = AMBIENT + (((ONE - AMBIENT) * light) >> SHIFT);
    let bright = sqrt(lit);
    let ch = |shift: u32| {
        let full = ((LIT >> shift) & 0xff) as i32;
        (((full * bright + ONE / 2) >> SHIFT).min(255) as u32) << shift
    };
    ch(16) | ch(8) | ch(0)
}

/// A box of pixels, both ends included.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Box {
    pub x0: i32,
    pub y0: i32,
    pub x1: i32,
    pub y1: i32,
}

impl Box {
    pub const SCREEN: Box = Box {
        x0: 0,
        y0: 0,
        x1: W - 1,
        y1: H - 1,
    };
}

/// A rectangle entry: kind 1, the colour, and its box `dy` rows down.
pub fn rect(colour: u32, b: Box, dy: i32) -> [u32; WORDS] {
    let mut w = [0u32; WORDS];
    w[0] = 1 | (colour << 2);
    w[1] = (b.x0 as u32) | (((b.y0 + dy) as u32) << 16);
    w[2] = (b.x1 as u32) | (((b.y1 + dy) as u32) << 16);
    w
}

/// A vertex as the list carries it: sixteen bits each of x and y.
fn pair(p: [i32; 2]) -> u32 {
    ((p[0] as u32) & 0xffff) | (((p[1] as u32) & 0xffff) << 16)
}

/// A flat triangle entry, `dy` rows down: the box its corners span,
/// clipped to the frame, and the corners wound so that inside is where
/// no edge function is negative. `None` when nothing of it is on the
/// screen.
fn triangle(
    colour: u32,
    p: [i32; 2],
    q: [i32; 2],
    r: [i32; 2],
    dy: i32,
) -> Option<[u32; WORDS]> {
    let (p, mut q, mut r) = (
        [p[0], p[1] + dy * 16],
        [q[0], q[1] + dy * 16],
        [r[0], r[1] + dy * 16],
    );
    let area = (q[0] - p[0]) as i64 * (r[1] - p[1]) as i64
        - (q[1] - p[1]) as i64 * (r[0] - p[0]) as i64;
    if area < 0 {
        core::mem::swap(&mut q, &mut r);
    }
    let lo = |a: i32, b: i32, c: i32| a.min(b).min(c) >> 4;
    let hi = |a: i32, b: i32, c: i32| (a.max(b).max(c) + 15) >> 4;
    let x0 = lo(p[0], q[0], r[0]).max(0);
    let y0 = lo(p[1], q[1], r[1]).max(dy);
    let x1 = hi(p[0], q[0], r[0]).min(W - 1);
    let y1 = hi(p[1], q[1], r[1]).min(dy + H - 1);
    if x1 < x0 || y1 < y0 {
        return None;
    }
    let mut w = [0u32; WORDS];
    w[0] = 2 | (colour << 2);
    w[1] = (x0 as u32) | ((y0 as u32) << 16);
    w[2] = (x1 as u32) | ((y1 as u32) << 16);
    w[3] = pair(p);
    w[4] = pair(q);
    w[5] = pair(r);
    Some(w)
}

/// One frame's list, into `out`, for the frame `dy` rows down (0 or
/// [`SECOND`]), with the solid turned by `ay` and `ax`. `clear` is the
/// box the solid filled the last time this frame was drawn. Returns
/// the entries written and the box the solid fills now, which is the
/// next `clear` for this frame.
pub fn frame(
    solid: &Solid,
    ay: i32,
    ax: i32,
    dy: i32,
    clear: Box,
    out: &mut [[u32; WORDS]; MOST],
) -> (usize, Box) {
    let mut at = [[0i32; 2]; VERTS];
    let mut v = 0;
    let mut b = Box {
        x0: W,
        y0: H,
        x1: -1,
        y1: -1,
    };
    while v < VERTS {
        let s = project(turn(BODY[v], ay, ax));
        at[v] = s;
        b.x0 = b.x0.min(s[0] >> 4);
        b.y0 = b.y0.min(s[1] >> 4);
        b.x1 = b.x1.max((s[0] + 15) >> 4);
        b.y1 = b.y1.max((s[1] + 15) >> 4);
        v += 1;
    }
    let b = Box {
        x0: b.x0.max(0),
        y0: b.y0.max(0),
        x1: b.x1.min(W - 1),
        y1: b.y1.min(H - 1),
    };
    out[0] = rect(BACKDROP, clear, dy);
    let mut n = 1;
    let mut f = 0;
    while f < solid.found {
        let [i, j, k] = solid.face[f];
        let (p, q, r) = (at[i], at[j], at[k]);
        // A face wound outwards turns anticlockwise to a viewer in
        // front of it, and the screen's rows run down, so a face that
        // faces the camera is clockwise on the screen. That holds in
        // perspective, where the normal's own direction does not quite.
        let area = (q[0] - p[0]) as i64 * (r[1] - p[1]) as i64
            - (q[1] - p[1]) as i64 * (r[0] - p[0]) as i64;
        let facing = area < 0;
        if facing {
            // The light is at the viewer's eye: how directly the face
            // looks at the camera is how lit it is.
            let light = turn(solid.normal[f], ay, ax)[2];
            if let Some(w) = triangle(shade(light), p, q, r, dy) {
                out[n] = w;
                n += 1;
            }
        }
        f += 1;
    }
    (n, b)
}
