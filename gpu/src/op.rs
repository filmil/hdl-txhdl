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
    /// Fill a triangle, in either winding.
    Tri {
        colour: u32,
        a: (i32, i32),
        b: (i32, i32),
        c: (i32, i32),
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
    /// edge function is non-negative. Two's complement, so a vertex
    /// may lie off the screen on any side; see [`VMIN`].
    pub ax: U<12>,
    pub ay: U<12>,
    pub bx: U<12>,
    pub by: U<12>,
    pub cx: U<12>,
    pub cy: U<12>,
}
// end{op}

/// The range a vertex may take. A vertex is twelve bits of two's
/// complement, and the rasteriser's edge arithmetic is thirty-two
/// bits, which is wide enough for every product of two differences
/// of vertices in this range.
pub const VMIN: i32 = -2048;
pub const VMAX: i32 = 2047;

/// A vertex as it is stored: twelve bits of two's complement.
pub fn vertex(v: i32) -> U<12> {
    U::from((v & 0xfff) as u32)
}

/// A stored vertex read back as a number.
pub fn signed(v: U<12>) -> i32 {
    let r = v.raw() as i32;
    if r >= 2048 {
        r - 4096
    } else {
        r
    }
}

/// Twice the signed area of the triangle `a`, `b`, `c`: positive when
/// the three are wound the way the rasteriser wants.
fn area2(a: (i32, i32), b: (i32, i32), c: (i32, i32)) -> i32 {
    (b.0 - a.0) * (c.1 - a.1) - (b.1 - a.1) * (c.0 - a.0)
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
                // The winding the rasteriser wants: swap two vertices
                // when the signed area says the other way.
                let (b, c) = if area2(a, b, c) < 0 { (c, b) } else { (b, c) };
                if area2(a, b, c) == 0 {
                    return None;
                }
                // Every vertex must be in range, since the edge
                // arithmetic is sized for that range and no wider.
                let ok = |p: (i32, i32)| {
                    (VMIN..=VMAX).contains(&p.0) && (VMIN..=VMAX).contains(&p.1)
                };
                if !ok(a) || !ok(b) || !ok(c) {
                    return None;
                }
                let lo = |f: fn((i32, i32)) -> i32| f(a).min(f(b)).min(f(c));
                let hi = |f: fn((i32, i32)) -> i32| f(a).max(f(b)).max(f(c));
                let (x0, y0, x1, y1) = clip(
                    lo(|p| p.0),
                    lo(|p| p.1),
                    hi(|p| p.0),
                    hi(|p| p.1),
                    sw,
                    sh,
                )?;
                Some(Insn {
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
                })
            }
        }
    }
}

// end{encode}

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
