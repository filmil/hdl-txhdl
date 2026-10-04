// SPDX-License-Identifier: Apache-2.0
//! The display list: what the rasteriser is told to draw, and how it
//! is encoded.
//!
//! [`Op`] is the instruction set as a program writes it, and its three
//! entries carry different things: a clear carries a colour and
//! nothing else, a rectangle carries its box, and a triangle carries
//! three vertices. [`Insn`] is what goes on the wire, and it is one
//! shape, as wide as the widest entry needs. [`Op::encode`] is the
//! assembler between them, and it does what a host can do once per
//! primitive rather than once per pixel: it clips the box to the
//! screen and winds the triangle so that its inside is where all
//! three edge functions are non-negative.
//!
//! The clear is the one entry whose box the hardware supplies, since
//! a clear is the whole screen by definition and the rasteriser knows
//! the screen's size from its type. That is the difference between
//! the three that costs the decoder anything, and it is the reason
//! this is worth writing as an instruction set at all.
use txhdl::types::U;
use txhdl::{Transaction as TransactionDerive, Value as ValueDerive};

// begin{op}
/// A display list entry, as a program writes it. Coordinates are in
/// pixels and may lie off the screen; the encoder clips.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Op {
    /// Fill the screen.
    Clear { colour: u32 },
    /// Fill a rectangle `w` by `h` pixels at `x`, `y`.
    Rect {
        colour: u32,
        x: i32,
        y: i32,
        w: i32,
        h: i32,
    },
    /// Fill a triangle, in either winding, its vertices in whole
    /// pixels.
    Tri {
        colour: u32,
        a: (i32, i32),
        b: (i32, i32),
        c: (i32, i32),
    },
    /// The same with its vertices in sixteenths of a pixel, the
    /// precision the rasteriser keeps: `(16, 8)` is a pixel across and
    /// half a pixel down.
    TriQ4 {
        colour: u32,
        a: (i32, i32),
        b: (i32, i32),
        c: (i32, i32),
    },
    /// A triangle with a colour at each vertex, `0xRRGGBB`, blended
    /// across it: Gouraud shading. Vertices in sixteenths of a pixel.
    Gouraud {
        a: (i32, i32),
        b: (i32, i32),
        c: (i32, i32),
        colours: [u32; 3],
    },
}

/// Which entry an instruction is. The rasteriser reads this and
/// nothing else to know where its box comes from and whether to test
/// the edges.
#[derive(ValueDerive, Clone, Copy, Default, Debug, PartialEq, Eq)]
pub enum Kind {
    #[default]
    Clear,
    Rect,
    Tri,
    Shaded,
}

/// An entry as it goes to the rasteriser: one shape, as wide as the
/// widest entry needs, so that every field a step reads is at a fixed
/// place. A clear leaves the box and the vertices at zero and a
/// rectangle leaves the vertices at zero; the decoder reads only what
/// the kind says is there.
#[derive(TransactionDerive, ValueDerive, Clone, Copy, Default, Debug)]
pub struct Insn {
    pub kind: Kind,
    /// The colour written, as `0xRRGGBB`. Every entry has one.
    pub colour: U<24>,
    /// The box to walk, both ends included, clipped to the screen. A
    /// rectangle's and a triangle's; a clear's comes from the
    /// rasteriser's own screen size.
    pub x0: U<10>,
    pub y0: U<10>,
    pub x1: U<10>,
    pub y1: U<10>,
    /// A triangle's vertices, wound so that its inside is where every
    /// edge function is non-negative. Sixteenths of a pixel in two's
    /// complement, so a vertex may lie between pixels and off the
    /// screen on any side; see [`VMIN`].
    pub ax: U<16>,
    pub ay: U<16>,
    pub bx: U<16>,
    pub by: U<16>,
    pub cx: U<16>,
    pub cy: U<16>,
    /// A shaded triangle's three channels, red, green and blue, each a
    /// plane: its value at the centre of the box's first pixel, and
    /// what it gains a pixel to the right and a row down. Sixteen bits
    /// of fraction in thirty-two of two's complement. The assembler
    /// works them out, once a triangle; the rasteriser only adds.
    pub r0: U<32>,
    pub rdx: U<32>,
    pub rdy: U<32>,
    pub g0: U<32>,
    pub gdx: U<32>,
    pub gdy: U<32>,
    pub b0: U<32>,
    pub bdx: U<32>,
    pub bdy: U<32>,
}
// end{op}

/// Sixteenths of a pixel: the bits of a vertex below the pixel.
pub const SUB_BITS: u32 = 4;
pub const SUB: i32 = 1 << SUB_BITS;

/// The range a vertex may take, in sixteenths of a pixel: 1024 pixels
/// either side of the origin. The rasteriser's edge arithmetic is
/// thirty-two bits, and a product of two differences of vertices in
/// this range, measured at a pixel of a screen of up to 1024, is at
/// most 2^30, so two of them and their difference fit.
pub const VMIN: i32 = -1024 * SUB;
pub const VMAX: i32 = 1024 * SUB - 1;

/// A vertex as it is stored: sixteen bits of two's complement, in
/// sixteenths of a pixel.
pub fn vertex(v: i32) -> U<16> {
    U::from((v & 0xffff) as u32)
}

/// A stored vertex read back as a number of sixteenths.
pub fn signed(v: U<16>) -> i32 {
    v.raw() as u16 as i16 as i32
}

/// Twice the signed area of the triangle `a`, `b`, `c`: positive when
/// the three are wound the way the rasteriser wants.
fn area2(a: (i32, i32), b: (i32, i32), c: (i32, i32)) -> i64 {
    let d =
        |p: (i32, i32), q: (i32, i32)| ((q.0 - p.0) as i64, (q.1 - p.1) as i64);
    let ((bx, by), (cx, cy)) = (d(a, b), d(a, c));
    bx * cy - by * cx
}

// begin{encode}
impl Op {
    /// The instruction this entry encodes to on a screen of `sw` by
    /// `sh` pixels, or `None` when there is nothing to draw: a box
    /// wholly off the screen, a rectangle with no pixels in it, or a
    /// triangle with no area. The assembler does the clipping and the
    /// winding so that the rasteriser does neither.
    pub fn encode(&self, sw: usize, sh: usize) -> Option<Insn> {
        match *self {
            // A clear says only its colour. The box is the screen,
            // and the rasteriser supplies it.
            Op::Clear { colour } => Some(Insn {
                kind: Kind::Clear,
                colour: U::from(colour),
                ..Insn::default()
            }),
            Op::Rect { colour, x, y, w, h } => {
                let (x0, y0, x1, y1) =
                    clip(x, y, x + w - 1, y + h - 1, sw, sh)?;
                Some(Insn {
                    kind: Kind::Rect,
                    colour: U::from(colour),
                    x0: U::from(x0),
                    y0: U::from(y0),
                    x1: U::from(x1),
                    y1: U::from(y1),
                    ..Insn::default()
                })
            }
            Op::Tri { colour, a, b, c } => {
                let q = |p: (i32, i32)| (p.0 * SUB, p.1 * SUB);
                Op::TriQ4 {
                    colour,
                    a: q(a),
                    b: q(b),
                    c: q(c),
                }
                .encode(sw, sh)
            }
            Op::TriQ4 { colour, a, b, c } => {
                triangle(colour, a, b, c, None, sw, sh)
            }
            Op::Gouraud { a, b, c, colours } => {
                triangle(colours[0], a, b, c, Some(colours), sw, sh)
            }
        }
    }
}

/// A triangle in sixteenths of a pixel: flat in `colour`, or shaded
/// from a colour at each vertex.
fn triangle(
    colour: u32,
    a: (i32, i32),
    b: (i32, i32),
    c: (i32, i32),
    shades: Option<[u32; 3]>,
    sw: usize,
    sh: usize,
) -> Option<Insn> {
    // The winding the rasteriser wants: swap two vertices, and their
    // colours, when the signed area says the other way.
    let swap = area2(a, b, c) < 0;
    let (b, c) = if swap { (c, b) } else { (b, c) };
    let shades = shades.map(|s| if swap { [s[0], s[2], s[1]] } else { s });
    if area2(a, b, c) == 0 {
        return None;
    }
    // Every vertex must be in range, since the edge arithmetic is
    // sized for that range and no wider.
    let ok = |p: (i32, i32)| {
        (VMIN..=VMAX).contains(&p.0) && (VMIN..=VMAX).contains(&p.1)
    };
    if !ok(a) || !ok(b) || !ok(c) {
        return None;
    }
    // The box: every pixel whose centre the triangle could cover, from
    // the pixel the lowest vertex is in to the pixel the highest is in.
    let px = |v: i32| v.div_euclid(SUB);
    let lo = |f: fn((i32, i32)) -> i32| px(f(a).min(f(b)).min(f(c)));
    let hi = |f: fn((i32, i32)) -> i32| px(f(a).max(f(b)).max(f(c)));
    let (x0, y0, x1, y1) =
        clip(lo(|p| p.0), lo(|p| p.1), hi(|p| p.0), hi(|p| p.1), sw, sh)?;
    let mut insn = Insn {
        kind: Kind::Tri,
        colour: U::from(colour),
        x0: U::from(x0),
        y0: U::from(y0),
        x1: U::from(x1),
        y1: U::from(y1),
        ax: vertex(a.0),
        ay: vertex(a.1),
        bx: vertex(b.0),
        by: vertex(b.1),
        cx: vertex(c.0),
        cy: vertex(c.1),
        ..Insn::default()
    };
    if let Some(s) = shades {
        insn.kind = Kind::Shaded;
        let first = (x0 as i32 * SUB + SUB / 2, y0 as i32 * SUB + SUB / 2);
        let [r, g, bl] = [16, 8, 0].map(|at| {
            let ch = |v: u32| ((v >> at) & 0xff) as i64;
            plane(a, b, c, [ch(s[0]), ch(s[1]), ch(s[2])], first)
        });
        (insn.r0, insn.rdx, insn.rdy) = r;
        (insn.g0, insn.gdx, insn.gdy) = g;
        (insn.b0, insn.bdx, insn.bdy) = bl;
    }
    Some(insn)
}
// end{encode}

/// One channel's plane across a triangle wound as the rasteriser wants,
/// with values `v` at its vertices: the value at `first`, the centre of
/// the box's first pixel, and what it gains a pixel right and a row
/// down, each with sixteen bits of fraction, rounded to the nearest.
/// The start carries half a unit more, so that the byte the rasteriser
/// takes above the fraction, which drops it, is the nearest one.
fn plane(
    a: (i32, i32),
    b: (i32, i32),
    c: (i32, i32),
    v: [i64; 3],
    first: (i32, i32),
) -> (U<32>, U<32>, U<32>) {
    let d = |p: (i32, i32), q: (i32, i32)| {
        ((q.0 - p.0) as i128, (q.1 - p.1) as i128)
    };
    let ((ux, uy), (vx, vy)) = (d(a, b), d(a, c));
    let area = ux * vy - uy * vx;
    let (db, dc) = ((v[1] - v[0]) as i128, (v[2] - v[0]) as i128);
    // The gradient, per sixteenth of a pixel, is (nx, ny) / area.
    let nx = db * vy - dc * uy;
    let ny = dc * ux - db * vx;
    let one = 1i128 << 16;
    let round = |n: i128| (n + area / 2).div_euclid(area);
    let (px, py) = d(a, first);
    let start = v[0] as i128 * one + one / 2 + round((nx * px + ny * py) * one);
    let word = |x: i128| U::<32>::from(x as i64 as i32 as u32);
    (
        word(start),
        word(round(nx * SUB as i128 * one)),
        word(round(ny * SUB as i128 * one)),
    )
}

/// A display list assembled: every entry that draws something, in
/// order, as the instructions the rasteriser reads.
pub fn assemble(ops: &[Op], sw: usize, sh: usize) -> Vec<Insn> {
    ops.iter().filter_map(|o| o.encode(sw, sh)).collect()
}

/// A box clipped to the screen, both ends included, or `None` when
/// nothing of it is on the screen.
fn clip(
    x0: i32,
    y0: i32,
    x1: i32,
    y1: i32,
    sw: usize,
    sh: usize,
) -> Option<(u32, u32, u32, u32)> {
    let (x0, y0) = (x0.max(0), y0.max(0));
    let (x1, y1) = (x1.min(sw as i32 - 1), y1.min(sh as i32 - 1));
    if x1 < x0 || y1 < y0 {
        return None;
    }
    Some((x0 as u32, y0 as u32, x1 as u32, y1 as u32))
}
