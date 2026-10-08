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
use txhdl::types::{Bit, U};
use txhdl::{Transaction as TransactionDerive, Value as ValueDerive};

// begin{op}
/// A display list entry, as a program writes it. Coordinates are in
/// pixels and may lie off the screen; the encoder clips. A colour is
/// `0xAARRGGBB`, and its alpha goes into the pixel's top byte as it is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Op {
    /// Fill the screen, or the scissor box when one is set.
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
    /// A triangle with a colour at each vertex, blended across it:
    /// Gouraud shading. Vertices in sixteenths of a pixel. Red, green
    /// and blue are blended; the alpha is the first vertex's, all over.
    Gouraud {
        a: (i32, i32),
        b: (i32, i32),
        c: (i32, i32),
        colours: [u32; 3],
    },
    /// From here on, draw only inside a box `w` by `h` pixels at `x`,
    /// `y`: the scissor box. A box that holds the screen turns it off.
    /// It is state the assembler keeps, and draws nothing itself.
    Scissor { x: i32, y: i32, w: i32, h: i32 },
    /// From here on, test and write depth as `mode` says, or not at all
    /// with `None` (issue 992). State, as the scissor box is: only the
    /// entries below that carry a depth, `RectZ`, `TriZ` and `GouraudZ`,
    /// are tested, and only in a tiled list, since depth lives only in
    /// Razboj's tile buffer; in a flat list they draw as if depth were
    /// off.
    Depth(Option<DepthMode>),
    /// A rectangle at the depth `z`, the same everywhere: what a clear
    /// of the depth buffer is, with [`ALWAYS`] and depth written.
    RectZ {
        colour: u32,
        x: i32,
        y: i32,
        w: i32,
        h: i32,
        z: u32,
    },
    /// A flat triangle with a depth at each vertex, `0` the nearest and
    /// `0xffff` the farthest, its vertices in sixteenths of a pixel.
    TriZ {
        colour: u32,
        a: (i32, i32),
        b: (i32, i32),
        c: (i32, i32),
        z: [u32; 3],
    },
    /// A shaded triangle with a depth at each vertex.
    GouraudZ {
        a: (i32, i32),
        b: (i32, i32),
        c: (i32, i32),
        colours: [u32; 3],
        z: [u32; 3],
    },
    /// From here on, blend as `mode` says, or not at all with `None`
    /// (issue 993). State, as the depth mode is, and like depth it holds
    /// only in a tiled list.
    Blend(Option<BlendMode>),
    /// From here on, drop a pixel whose alpha fails the test, or test
    /// nothing with `None` (issue 993). Tiled lists only.
    AlphaTest(Option<AlphaTest>),
    /// From here on, write only the channels `mask` holds, a bit a byte
    /// of the pixel: bit 0 blue, 1 green, 2 red, 3 alpha, so `0xf` is
    /// every channel and nought none (issue 993). Tiled lists only.
    ColourMask(u32),
    /// From here on, write each pixel as GL's logic operation `op` makes
    /// it from the pixel and the framebuffer's, bit by bit, in place of
    /// the blend, or not with `None` (issue 998): `op` is GL's `GL_CLEAR`
    /// to `GL_SET` less `0x1500`, see [`logic`]. Tiled lists only.
    LogicOp(Option<u32>),
    /// From here on, fog every entry but a clear as `fog` says, or none
    /// with `None` (issue 998): after its texture and before the alpha
    /// test, as GL orders it. Tiled lists only.
    Fog(Option<Fog>),
    /// From here on, texture the entries that carry texture coordinates
    /// as `mode` says, or not at all with `None` (issue 997). Tiled lists
    /// only.
    Texture(Option<TexMode>),
    /// A triangle with texture coordinates: flat in `colours[0]`, or
    /// shaded from a colour at each vertex when `shaded`; a depth at each
    /// vertex, used under a depth mode; and at each vertex `u q`, `v q`
    /// and `q`, with 32, 32 and 48 bits of fraction (see
    /// `razboj_tile::tex`). The assembler works out the level of detail's
    /// planes from them.
    TexTri {
        a: (i32, i32),
        b: (i32, i32),
        c: (i32, i32),
        colours: [u32; 3],
        shaded: bool,
        z: [u32; 3],
        uvq: [(i64, i64, u64); 3],
    },
}

/// How entries are textured (issue 997): the byte address of the texture's
/// descriptor, the environment, `razboj_tile::tex::REPLACE` to `ADD`, and
/// the environment's colour, `0xAARRGGBB`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TexMode {
    pub desc: u32,
    pub env: u32,
    pub env_colour: u32,
}

/// How an entry blends (issue 993): GL ES 1.1's `glBlendFunc`, the
/// source's factor and the destination's, as [`ZERO`] to
/// [`SRC_ALPHA_SATURATE`] say them, under the one equation 1.1 has, the
/// sum.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BlendMode {
    pub src: u32,
    pub dst: u32,
}

/// GL's blend factors, numbered for the four bits the list gives each:
/// `GL_ZERO` and `GL_ONE` as they are, and from `GL_SRC_COLOR` on, GL's
/// value less `0x300` and plus two. [`SRC_ALPHA_SATURATE`] is for the
/// source only.
pub const ZERO: u32 = 0;
pub const ONE: u32 = 1;
pub const SRC_COLOR: u32 = 2;
pub const ONE_MINUS_SRC_COLOR: u32 = 3;
pub const SRC_ALPHA: u32 = 4;
pub const ONE_MINUS_SRC_ALPHA: u32 = 5;
pub const DST_ALPHA: u32 = 6;
pub const ONE_MINUS_DST_ALPHA: u32 = 7;
pub const DST_COLOR: u32 = 8;
pub const ONE_MINUS_DST_COLOR: u32 = 9;
pub const SRC_ALPHA_SATURATE: u32 = 10;

/// GL's logic operations (issue 998), `GL_CLEAR` to `GL_SET` less
/// `0x1500`. GL numbers them so that each is a truth table: bit `3 - (2s
/// + d)` of the operation is the result for a source bit `s` and a
/// destination bit `d`.
pub const CLEAR: u32 = 0;
pub const AND: u32 = 1;
pub const COPY: u32 = 3;
pub const NOOP: u32 = 5;
pub const XOR: u32 = 6;
pub const OR: u32 = 7;
pub const INVERT: u32 = 10;
pub const NAND: u32 = 14;
pub const SET: u32 = 15;

/// The pixel the logic operation `op` makes of the source `s` and the
/// destination `d`, bit by bit, every channel and alpha alike.
pub fn logic(op: u32, s: u32, d: u32) -> u32 {
    let take = |k: u32| if (op >> k) & 1 == 1 { u32::MAX } else { 0 };
    (take(3) & !s & !d)
        | (take(2) & !s & d)
        | (take(1) & s & !d)
        | (take(0) & s & d)
}

/// Fog (issue 998): its colour, `0xRRGGBB`, and GL's fog factor `f` at
/// each vertex of the entries it fogs, as a byte, 255 for GL's one, no
/// fog, and nought for all fog. A rectangle takes the first. Each of red,
/// green and blue becomes `f c + (255 - f) fog` over 255, and the alpha
/// is kept.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Fog {
    pub colour: u32,
    pub f: [u32; 3],
}

/// An alpha test (issue 993): a pixel is kept when its alpha passes the
/// comparison `func`, [`NEVER`] to [`ALWAYS`] as depth's, against
/// `reference`, a byte.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AlphaTest {
    pub func: u32,
    pub reference: u32,
}

/// How an entry tests depth: the comparison a pixel's depth must pass
/// against the depth already there, GL's eight in GL's order, and
/// whether a pixel that passes writes its depth.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DepthMode {
    pub func: u32,
    pub write: bool,
}

/// The comparisons, as `DepthMode::func` and word 15's bits 11 to 9 say
/// them: GL's `GL_NEVER` to `GL_ALWAYS`, less `0x0200`.
pub const NEVER: u32 = 0;
pub const LESS: u32 = 1;
pub const EQUAL: u32 = 2;
pub const LEQUAL: u32 = 3;
pub const GREATER: u32 = 4;
pub const NOTEQUAL: u32 = 5;
pub const GEQUAL: u32 = 6;
pub const ALWAYS: u32 = 7;

/// Bits of fraction in a depth plane: twelve, so that the sixteen bits
/// of depth and the sign fit in thirty-two.
pub const ZFRAC: u32 = 12;

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
    /// The alpha written, in the pixel's top byte. Every entry has one,
    /// and nothing yet reads it back.
    pub alpha: U<8>,
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
    /// Depth (issue 992): whether the entry tests it, the comparison,
    /// and whether a pixel that passes writes its depth; then the depth
    /// plane, as a channel's, with [`ZFRAC`] bits of fraction. An entry
    /// that tests depth takes a second slot in the list for its plane.
    pub depth: Bit,
    pub zfunc: U<3>,
    pub zwrite: Bit,
    pub z0: U<32>,
    pub zdx: U<32>,
    pub zdy: U<32>,
    /// Blending, the alpha test and the colour mask (issue 993): whether
    /// the entry has any of them, which also gives it the second slot;
    /// whether it blends, and its two factors; whether it tests alpha,
    /// the comparison and the reference; and the channels it writes, a
    /// bit a byte. Without `state` an entry blends nothing, tests no
    /// alpha and writes every channel, whatever the rest say.
    pub state: Bit,
    pub blend: Bit,
    pub sfactor: U<4>,
    pub dfactor: U<4>,
    pub atest: Bit,
    pub afunc: U<3>,
    pub aref: U<8>,
    pub cmask: U<4>,
    /// The logic operation (issue 998): whether it is on, in place of the
    /// blend, and which of GL's sixteen, as [`logic`] reads it.
    pub logic: Bit,
    pub lop: U<4>,
    /// Fog (issue 998): whether the entry is fogged, the fog's colour as
    /// `0xRRGGBB`, and the plane of its factor, as a channel's, with
    /// sixteen bits of fraction. An entry with fog has the pixel's state.
    pub fog: Bit,
    pub fcol: U<24>,
    pub f0: U<32>,
    pub fdx: U<32>,
    pub fdy: U<32>,
    /// Texturing (issue 997): whether the entry is textured, which gives
    /// it two slots more; its texture's descriptor's byte address, its
    /// environment and environment colour; the planes `u q`, `v q` and
    /// `q`, 64 bits each, which a triangle with texture coordinates
    /// carries whether textured or not; and for the level of detail, the
    /// numerators of `du/dx`, `dv/dx`, `du/dy` and `dv/dy`, each a value at
    /// the box's first pixel and a step, down for the first two and right
    /// for the others, since that is all each varies with, shifted right by
    /// `lodk`; see `crate::tex::lod`.
    pub tex: Bit,
    pub tdesc: U<32>,
    pub tenv: U<3>,
    pub tenvc: U<32>,
    pub lodk: U<8>,
    pub nux: U<32>,
    pub nuxd: U<32>,
    pub nvx: U<32>,
    pub nvxd: U<32>,
    pub nuy: U<32>,
    pub nuyd: U<32>,
    pub nvy: U<32>,
    pub nvyd: U<32>,
    pub u0: U<64>,
    pub udx: U<64>,
    pub udy: U<64>,
    pub v0: U<64>,
    pub vdx: U<64>,
    pub vdy: U<64>,
    pub q0: U<64>,
    pub qdx: U<64>,
    pub qdy: U<64>,
}

impl Insn {
    /// The channels the entry writes, a bit a byte of the pixel.
    pub fn mask(&self) -> u32 {
        if self.state.to_bool() {
            self.cmask.raw() as u32
        } else {
            0xf
        }
    }

    /// Whether the entry reads the colour already there: it blends, has a
    /// logic operation (issue 998), or writes some channels but not all
    /// (issue 993).
    pub fn reads_dst(&self) -> bool {
        let m = self.mask();
        let mixes = self.blend.to_bool() || self.logic.to_bool();
        self.state.to_bool() && (mixes || (m != 0 && m != 0xf))
    }
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

/// A box of pixels, both ends included, and its clip, which the scissor
/// box and the binning into tiles share.
pub use razboj_tile::{clip, Bounds};

/// The whole of a screen of `sw` by `sh` pixels.
pub fn screen(sw: usize, sh: usize) -> Bounds {
    (0, 0, sw as u32 - 1, sh as u32 - 1)
}

// begin{encode}
impl Op {
    /// The instruction this entry encodes to on a screen of `sw` by
    /// `sh` pixels with no scissor box, or `None` when there is nothing
    /// to draw: a box wholly off the screen, a rectangle with no pixels
    /// in it, a triangle with no area, or a scissor box, which is state.
    /// The assembler does the clipping and the winding so that the
    /// rasteriser does neither.
    pub fn encode(&self, sw: usize, sh: usize) -> Option<Insn> {
        self.encode_in(screen(sw, sh), sw, sh)
    }

    /// The same, drawn only inside `within`, which lies on the screen:
    /// the scissor box. The rasteriser walks only an entry's box, so
    /// clipping the box to the scissor box is the whole of the scissor
    /// test, done once an entry rather than once a pixel.
    pub fn encode_in(
        &self,
        within: Bounds,
        sw: usize,
        sh: usize,
    ) -> Option<Insn> {
        let shade = |c: u32| (U::from(c & 0xff_ffff), U::from(c >> 24));
        match *self {
            // A clear says only its colour. The box is the screen,
            // and the rasteriser supplies it. Under a scissor box it is
            // a rectangle of that box.
            Op::Clear { colour } if within == screen(sw, sh) => {
                let (colour, alpha) = shade(colour);
                Some(Insn {
                    kind: Kind::Clear,
                    colour,
                    alpha,
                    ..Insn::default()
                })
            }
            Op::Clear { colour } => {
                let (x0, y0, x1, y1) = within;
                let (x, y) = (x0 as i32, y0 as i32);
                let (w, h) = ((x1 - x0 + 1) as i32, (y1 - y0 + 1) as i32);
                Op::Rect { colour, x, y, w, h }.encode_in(within, sw, sh)
            }
            Op::Rect { colour, x, y, w, h } => {
                let (x0, y0, x1, y1) =
                    clip(x, y, x + w - 1, y + h - 1, within)?;
                let (colour, alpha) = shade(colour);
                Some(Insn {
                    kind: Kind::Rect,
                    colour,
                    alpha,
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
                .encode_in(within, sw, sh)
            }
            Op::TriQ4 { colour, a, b, c } => {
                triangle(colour, [a, b, c], None, None, None, within)
            }
            Op::Gouraud { a, b, c, colours } => triangle(
                colours[0],
                [a, b, c],
                Some(colours),
                None,
                None,
                within,
            ),
            // Without a depth mode an entry with a depth draws as the
            // same entry without one.
            Op::RectZ {
                colour, x, y, w, h, ..
            } => Op::Rect { colour, x, y, w, h }.encode_in(within, sw, sh),
            Op::TriZ {
                colour, a, b, c, ..
            } => Op::TriQ4 { colour, a, b, c }.encode_in(within, sw, sh),
            Op::GouraudZ {
                a, b, c, colours, ..
            } => Op::Gouraud { a, b, c, colours }.encode_in(within, sw, sh),
            Op::TexTri {
                a,
                b,
                c,
                colours,
                shaded,
                uvq,
                ..
            } => {
                let s = shaded.then_some(colours);
                triangle(colours[0], [a, b, c], s, None, Some(uvq), within)
            }
            Op::Scissor { .. }
            | Op::Depth(_)
            | Op::Blend(_)
            | Op::AlphaTest(_)
            | Op::ColourMask(_)
            | Op::LogicOp(_)
            | Op::Fog(_)
            | Op::Texture(_) => None,
        }
    }

    /// The same under a depth mode (issue 992): an entry that carries a
    /// depth tests it as `depth` says, with its depth plane; every other
    /// entry is as [`Op::encode_in`] gives it.
    pub fn encode_with(
        &self,
        within: Bounds,
        depth: Option<DepthMode>,
        sw: usize,
        sh: usize,
    ) -> Option<Insn> {
        let Some(mode) = depth else {
            return self.encode_in(within, sw, sh);
        };
        let mut insn = match *self {
            Op::RectZ { z, .. } => {
                let mut i = self.encode_in(within, sw, sh)?;
                // The same depth everywhere, half a unit up as a
                // channel's start is.
                i.z0 = U::from((z << ZFRAC) + (1 << (ZFRAC - 1)));
                i
            }
            Op::TriZ { colour, a, b, c, z } => {
                triangle(colour, [a, b, c], None, Some(z), None, within)?
            }
            Op::GouraudZ {
                a,
                b,
                c,
                colours,
                z,
            } => triangle(
                colours[0],
                [a, b, c],
                Some(colours),
                Some(z),
                None,
                within,
            )?,
            Op::TexTri {
                a,
                b,
                c,
                colours,
                shaded,
                z,
                uvq,
            } => {
                let s = shaded.then_some(colours);
                let t = Some(uvq);
                triangle(colours[0], [a, b, c], s, Some(z), t, within)?
            }
            _ => return self.encode_in(within, sw, sh),
        };
        insn.depth = Bit::One;
        insn.zfunc = U::from(mode.func);
        insn.zwrite = Bit::from(mode.write);
        Some(insn)
    }
}

/// A triangle in sixteenths of a pixel, inside `within`: flat in
/// `colour`, or shaded from a colour at each vertex.
fn triangle(
    colour: u32,
    v: [(i32, i32); 3],
    shades: Option<[u32; 3]>,
    zs: Option<[u32; 3]>,
    uvq: Option<[(i64, i64, u64); 3]>,
    within: Bounds,
) -> Option<Insn> {
    let [a, b, c] = v;
    // The winding the rasteriser wants: swap two vertices, and their
    // colours and depths, when the signed area says the other way.
    let swap = area2(a, b, c) < 0;
    let (b, c) = if swap { (c, b) } else { (b, c) };
    let turn = |s: [u32; 3]| if swap { [s[0], s[2], s[1]] } else { s };
    let (shades, zs) = (shades.map(turn), zs.map(turn));
    let uvq = uvq.map(|t| if swap { [t[0], t[2], t[1]] } else { t });
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
        clip(lo(|p| p.0), lo(|p| p.1), hi(|p| p.0), hi(|p| p.1), within)?;
    let mut insn = Insn {
        kind: Kind::Tri,
        colour: U::from(colour & 0xff_ffff),
        alpha: U::from(colour >> 24),
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
            plane(a, b, c, [ch(s[0]), ch(s[1]), ch(s[2])], first, 16)
        });
        (insn.r0, insn.rdx, insn.rdy) = r;
        (insn.g0, insn.gdx, insn.gdy) = g;
        (insn.b0, insn.bdx, insn.bdy) = bl;
    }
    if let Some(z) = zs {
        let first = (x0 as i32 * SUB + SUB / 2, y0 as i32 * SUB + SUB / 2);
        let v = z.map(|z| (z & 0xffff) as i64);
        (insn.z0, insn.zdx, insn.zdy) = plane(a, b, c, v, first, ZFRAC);
    }
    if let Some(t) = uvq {
        // The texture's planes in 64 bits, each rounded to the nearest, so
        // no half is added (issue 997).
        let first = (x0 as i32 * SUB + SUB / 2, y0 as i32 * SUB + SUB / 2);
        let w = |p: [i128; 3]| p.map(|v| U::<64>::from(v as i64 as u64));
        let r = recip(a, b, c);
        let pl = |v: [i128; 3]| plane64(a, b, c, v, first, r);
        let (u, v, q) = (
            pl(t.map(|t| t.0 as i128)),
            pl(t.map(|t| t.1 as i128)),
            pl(t.map(|t| t.2 as i128)),
        );
        [insn.u0, insn.udx, insn.udy] = w(u);
        [insn.v0, insn.vdx, insn.vdy] = w(v);
        [insn.q0, insn.qdx, insn.qdy] = w(q);
        // The level of detail's numerators. With `U = u q`, `du/dx` is
        // `(U_x Q - U Q_x) / Q^2`, whose numerator varies with the row and
        // not the column, and `du/dy`'s with the column and not the row:
        // each is a value at the box's first pixel and one step.
        let wrap = |p: [i128; 3]| p.map(|v| v as i64 as i128);
        let ([u0, ux, uy], [v0, vx, vy], [q0, qx, qy]) =
            (wrap(u), wrap(v), wrap(q));
        let n = [
            (ux * q0 - u0 * qx, ux * qy - uy * qx),
            (vx * q0 - v0 * qx, vx * qy - vy * qx),
            (uy * q0 - u0 * qy, uy * qx - ux * qy),
            (vy * q0 - v0 * qy, vy * qx - vx * qy),
        ];
        // Shifted right until each fits 31 bits across the box, so that
        // the walk steps them in 32: what `log2` needs of them.
        let (bw, bh) = ((x1 - x0) as i128, (y1 - y0) as i128);
        let far = n
            .iter()
            .enumerate()
            .map(|(k, &(v, d))| {
                let span = if k < 2 { bh } else { bw };
                v.abs().max((v + d * span).abs())
            })
            .max()
            .unwrap_or(0);
        let k = (128 - far.leading_zeros()).saturating_sub(30);
        let s = |x: i128| U::<32>::from((x >> k) as i32 as u32);
        insn.lodk = U::from(k);
        (insn.nux, insn.nuxd) = (s(n[0].0), s(n[0].1));
        (insn.nvx, insn.nvxd) = (s(n[1].0), s(n[1].1));
        (insn.nuy, insn.nuyd) = (s(n[2].0), s(n[2].1));
        (insn.nvy, insn.nvyd) = (s(n[3].0), s(n[3].1));
    }
    Some(insn)
}

/// A plane in 64 bits for a texture (issue 997): its value at `first` and
/// its two steps, in the units the values `v` at the vertices are in,
/// rounded to the nearest.
///
/// Nothing is divided (#1433), since the core that writes a list does a
/// division of 128 bits in software: `r` is the reciprocal of the area,
/// which [`recip`] works out once for the three planes, and the
/// differences between the vertices keep their top 46 bits, so that the
/// products with the vertices' offsets fit 64 bits.
fn plane64(
    a: (i32, i32),
    b: (i32, i32),
    c: (i32, i32),
    v: [i128; 3],
    first: (i32, i32),
    r: (i128, u32),
) -> [i128; 3] {
    let d =
        |p: (i32, i32), q: (i32, i32)| ((q.0 - p.0) as i64, (q.1 - p.1) as i64);
    let ((ux, uy), (vx, vy)) = (d(a, b), d(a, c));
    let (db, dc) = (v[1] - v[0], v[2] - v[0]);
    let big = db.unsigned_abs().max(dc.unsigned_abs());
    let s = (128 - big.leading_zeros()).saturating_sub(46);
    let (db, dc) = ((db >> s) as i64, (dc >> s) as i64);
    let nx = db * vy - dc * uy;
    let ny = dc * ux - db * vx;
    // A pixel's steps with sixteen more bits of fraction, for the start.
    let (m, sh) = r;
    let step = |n: i64| shr(n as i128 * m, sh - SUB_BITS - 16);
    let (fx, fy) = (step(nx), step(ny));
    let (px, py) = d(a, first);
    let at = shr(fx * px as i128 + fy * py as i128, SUB_BITS + 16);
    [v[0] + (at << s), shr(fx, 16) << s, shr(fy, 16) << s]
}

/// The reciprocal of a triangle's area, twice it in sixteenths squared,
/// for [`plane64`]: `m` over two to the `sh` is within one part in
/// `2^62` of one over the area, and `m` is at most `2^63`. Two divisions
/// of 64 bits, the first of the top and the second of what is left.
fn recip(a: (i32, i32), b: (i32, i32), c: (i32, i32)) -> (i128, u32) {
    let area = area2(a, b, c) as u64;
    let l = 64 - area.leading_zeros();
    let top = 1u64 << (30 + l);
    let (q, rest) = (top / area, top % area);
    (((q << 32) + (rest << 32) / area) as i128, 62 + l)
}

/// `x` over two to the `k`, rounded to the nearest, a half up.
fn shr(x: i128, k: u32) -> i128 {
    (x + (1 << (k - 1))) >> k
}
// end{encode}

/// One channel's plane across a triangle wound as the rasteriser wants,
/// with values `v` at its vertices: the value at `first`, the centre of
/// the box's first pixel, and what it gains a pixel right and a row
/// down, each with `frac` bits of fraction, sixteen for a colour and
/// [`ZFRAC`] for a depth, rounded to the nearest. The start carries half
/// a unit more, so that the value the rasteriser takes above the
/// fraction, which drops it, is the nearest one.
fn plane(
    a: (i32, i32),
    b: (i32, i32),
    c: (i32, i32),
    v: [i64; 3],
    first: (i32, i32),
    frac: u32,
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
    let one = 1i128 << frac;
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
/// order, as the instructions the rasteriser reads. A scissor box holds
/// from where it is set to where the next is, and one wholly off the
/// screen draws nothing until then; a depth mode holds the same way.
pub fn assemble(ops: &[Op], sw: usize, sh: usize) -> Vec<Insn> {
    let mut within = Some(screen(sw, sh));
    let mut depth = None;
    let mut pixel = Pixel::default();
    let mut texture = None;
    let mut fog = None;
    let mut out = Vec::new();
    for op in ops {
        match *op {
            Op::Fog(f) => fog = f,
            Op::Scissor { x, y, w, h } => {
                within = clip(x, y, x + w - 1, y + h - 1, screen(sw, sh))
            }
            Op::Depth(mode) => depth = mode,
            Op::Blend(mode) => pixel.blend = mode,
            Op::AlphaTest(test) => pixel.alpha = test,
            Op::ColourMask(mask) => pixel.mask = mask & 0xf,
            Op::LogicOp(op) => pixel.logic = op.map(|o| o & 0xf),
            Op::Texture(mode) => texture = mode,
            _ => {
                if let Some(mut insn) =
                    within.and_then(|b| op.encode_with(b, depth, sw, sh))
                {
                    pixel.apply(&mut insn);
                    if let Some(f) =
                        fog.filter(|_| !matches!(op, Op::Clear { .. }))
                    {
                        fogged(&mut insn, op, f, pixel.mask);
                    }
                    if let (Some(t), Op::TexTri { .. }) = (texture, op) {
                        insn.tex = Bit::One;
                        insn.tdesc = U::from(t.desc);
                        insn.tenv = U::from(t.env);
                        insn.tenvc = U::from(t.env_colour);
                    }
                    out.push(insn);
                }
            }
        }
    }
    out
}

/// `insn`, encoded from `op`, fogged as `f` says (issue 998), with the
/// pixel's state and `mask` its channels: the factor's plane across a
/// triangle, its vertices' factors turned with them when the encoder
/// winds it the other way, or the first factor everywhere.
fn fogged(insn: &mut Insn, op: &Op, f: Fog, mask: u32) {
    let v = match *op {
        Op::Tri { a, b, c, .. } => {
            Some([a, b, c].map(|p| (p.0 * SUB, p.1 * SUB)))
        }
        Op::TriQ4 { a, b, c, .. }
        | Op::Gouraud { a, b, c, .. }
        | Op::TriZ { a, b, c, .. }
        | Op::GouraudZ { a, b, c, .. }
        | Op::TexTri { a, b, c, .. } => Some([a, b, c]),
        _ => None,
    };
    let fs = f.f.map(|f| (f & 0xff) as i64);
    (insn.f0, insn.fdx, insn.fdy) = match v {
        Some([a, b, c]) => {
            let swap = area2(a, b, c) < 0;
            let (b, c, fs) = if swap {
                (c, b, [fs[0], fs[2], fs[1]])
            } else {
                (b, c, fs)
            };
            let (x0, y0) = (insn.x0.raw() as i32, insn.y0.raw() as i32);
            let first = (x0 * SUB + SUB / 2, y0 * SUB + SUB / 2);
            plane(a, b, c, fs, first, 16)
        }
        None => {
            let z = U::from(0u8);
            (U::from(((fs[0] as u32) << 16) + (1 << 15)), z, z)
        }
    };
    if !insn.state.to_bool() {
        insn.state = Bit::One;
        insn.cmask = U::from(mask);
    }
    insn.fog = Bit::One;
    insn.fcol = U::from(f.colour & 0xff_ffff);
}

/// What happens to each pixel an entry draws after its coverage and
/// before its depth (issue 993): the blend, the alpha test and the
/// colour mask the assembler holds as state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Pixel {
    pub blend: Option<BlendMode>,
    pub alpha: Option<AlphaTest>,
    pub mask: u32,
    pub logic: Option<u32>,
}

impl Default for Pixel {
    fn default() -> Self {
        Pixel {
            blend: None,
            alpha: None,
            mask: 0xf,
            logic: None,
        }
    }
}

impl Pixel {
    /// `insn` with this state: none of it, if the state is GL's default,
    /// so that a list that uses none of it is as it was.
    pub fn apply(&self, insn: &mut Insn) {
        if *self == Pixel::default() {
            return;
        }
        insn.state = Bit::One;
        if let Some(b) = self.blend {
            insn.blend = Bit::One;
            insn.sfactor = U::from(b.src);
            insn.dfactor = U::from(b.dst);
        }
        if let Some(a) = self.alpha {
            insn.atest = Bit::One;
            insn.afunc = U::from(a.func);
            insn.aref = U::from(a.reference & 0xff);
        }
        insn.cmask = U::from(self.mask);
        if let Some(op) = self.logic {
            insn.logic = Bit::One;
            insn.lop = U::from(op);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Op, U};
    use crate::model::channel;

    /// A shaded triangle's planes give back the colour of each vertex at
    /// the pixel whose centre the vertex is, wound either way and so with
    /// its colours swapped by the encoder (issue 989).
    #[test]
    fn a_shaded_triangle_has_its_colours_at_its_vertices() {
        // The centres of pixels 1,1 and 14,2 and 3,14.
        let (a, b, c) = ((24, 24), (232, 40), (56, 232));
        let (ca, cb, cc) = (0xff_8000, 0x10_ff40, 0x30_20ff);
        for (b, c, colours) in [(b, c, [ca, cb, cc]), (c, b, [ca, cc, cb])] {
            let op = Op::Gouraud { a, b, c, colours };
            let i = op.encode(16, 16).expect("a triangle on the screen");
            let (x0, y0) = (i.x0.raw() as i32, i.y0.raw() as i32);
            let raw = |u: U<32>| u.raw() as u32;
            for (p, want) in [(a, colours[0]), (b, colours[1]), (c, colours[2])]
            {
                let (di, dj) = (p.0 / 16 - x0, p.1 / 16 - y0);
                let ch = |s, dx, dy| channel(raw(s), raw(dx), raw(dy), di, dj);
                let got = (ch(i.r0, i.rdx, i.rdy) << 16)
                    | (ch(i.g0, i.gdx, i.gdy) << 8)
                    | ch(i.b0, i.bdx, i.bdy);
                assert_eq!(got, want, "the vertex at {p:?}");
            }
        }
    }

    /// Fog's factor gives back each vertex's at the pixel whose centre the
    /// vertex is, wound either way, and a rectangle's is its first
    /// everywhere (issue 998).
    #[test]
    fn a_fogged_triangle_has_its_factors_at_its_vertices() {
        let (a, b, c) = ((24, 24), (232, 40), (56, 232));
        let colours = [0xff00_0000; 3];
        for (b, c, f) in [(b, c, [10, 128, 250]), (c, b, [10, 250, 128])] {
            let fog = super::Fog { colour: 0, f };
            let ops = [Op::Fog(Some(fog)), Op::Gouraud { a, b, c, colours }];
            let i = super::assemble(&ops, 16, 16)[0];
            let (x0, y0) = (i.x0.raw() as i32, i.y0.raw() as i32);
            let raw = |u: U<32>| u.raw() as u32;
            for (p, want) in [(a, f[0]), (b, f[1]), (c, f[2])] {
                let (di, dj) = (p.0 / 16 - x0, p.1 / 16 - y0);
                let got = channel(raw(i.f0), raw(i.fdx), raw(i.fdy), di, dj);
                assert_eq!(got, want, "the vertex at {p:?}");
            }
        }
        let fog = super::Fog {
            colour: 0x12_3456,
            f: [77, 0, 0],
        };
        let rect = Op::Rect {
            colour: 0,
            x: 2,
            y: 3,
            w: 4,
            h: 5,
        };
        let i = super::assemble(&[Op::Fog(Some(fog)), rect], 16, 16)[0];
        assert!(i.state.to_bool() && i.fog.to_bool());
        assert_eq!((i.cmask.raw(), i.fcol.raw()), (0xf, 0x12_3456));
        let raw = |u: U<32>| u.raw() as u32;
        assert_eq!(channel(raw(i.f0), raw(i.fdx), raw(i.fdy), 3, 4), 77);
        let clear = Op::Clear { colour: 0 };
        let i = super::assemble(&[Op::Fog(Some(fog)), clear], 16, 16)[0];
        assert!(!i.fog.to_bool(), "a clear is not fogged");
    }

    /// A texture's plane from the area's reciprocal is within two units
    /// of the plane divided out exactly, rounded to the nearest, when the
    /// vertices' values differ by under `2^46`. Past that the differences
    /// lose `s` bits, and each number may be out by what they lost, times
    /// the triangle's sides over its area for the start (#1433).
    #[test]
    fn a_texture_plane_is_within_two_units_of_exact() {
        let mut seed = 0x2545_f491_4f6c_dd1du64;
        let mut next = |n: u64| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed % n
        };
        let mut tried = 0;
        while tried < 4000 {
            let [ax, ay, bx, by, cx, cy] =
                [0; 6].map(|_| next(1 << 15) as i32 + super::VMIN);
            let (a, b, c) = ((ax, ay), (bx, by), (cx, cy));
            let area = super::area2(a, b, c) as i128;
            if area <= 0 {
                continue;
            }
            tried += 1;
            let bits: u32 = [20, 40, 45, 52, 63][tried % 5];
            let v = [0, 1, 2].map(|_| {
                (next(1 << bits) as i128 - (1 << (bits - 1))) as i64 as i128
            });
            let first = (a.0 + next(256) as i32, a.1 + next(256) as i32);
            let got = super::plane64(a, b, c, v, first, super::recip(a, b, c));
            let want = exact(a, b, c, v, first);
            let big = (v[1] - v[0]).abs().max((v[2] - v[0]).abs());
            let s = (128 - big.leading_zeros()).saturating_sub(46);
            let m = |p: (i32, i32), q: (i32, i32)| {
                ((q.0 - p.0).abs() + (q.1 - p.1).abs()) as i128
            };
            let sides = (m(a, b) + m(a, c)) * (m(a, first) + 16) / area;
            let off = if s == 0 { 2 } else { 2 + ((2 + sides) << s) };
            for (g, w) in got.iter().zip(want) {
                assert!(
                    (g - w).abs() <= off,
                    "{a:?} {b:?} {c:?} {v:?} {first:?}: {got:?}, {want:?}"
                );
            }
        }
    }

    /// The plane as it was worked out before #1433, with a division by
    /// the area for each of its three numbers.
    fn exact(
        a: (i32, i32),
        b: (i32, i32),
        c: (i32, i32),
        v: [i128; 3],
        first: (i32, i32),
    ) -> [i128; 3] {
        let d = |p: (i32, i32), q: (i32, i32)| {
            ((q.0 - p.0) as i128, (q.1 - p.1) as i128)
        };
        let ((ux, uy), (vx, vy)) = (d(a, b), d(a, c));
        let area = ux * vy - uy * vx;
        let (db, dc) = (v[1] - v[0], v[2] - v[0]);
        let nx = db * vy - dc * uy;
        let ny = dc * ux - db * vx;
        let round = |n: i128| (n + area / 2).div_euclid(area);
        let (px, py) = d(a, first);
        [
            v[0] + round(nx * px + ny * py),
            round(nx * super::SUB as i128),
            round(ny * super::SUB as i128),
        ]
    }
}
