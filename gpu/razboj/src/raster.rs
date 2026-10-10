// SPDX-License-Identifier: Apache-2.0
//! The rasteriser: one pixel a cycle, written into memory over AXI.
//!
//! It walks the box a display list entry gives it, one pixel per
//! cycle, and writes the entry's colour at every pixel that is in the
//! primitive. For a box that is every pixel; for a triangle it is
//! every pixel at which none of the three edge functions is negative.
//!
//! An edge function is linear, so stepping to the pixel on the right
//! adds a constant and stepping to the start of the next row adds
//! another. The rasteriser therefore keeps, per edge, the value at
//! the current pixel, the value at the start of the current row, and
//! the two steps; the only multiplications are the six of the setup,
//! done side by side in the second of the three cycles the setup takes
//! after the entry is fetched. This is what makes the per-pixel work
//! three adds and three sign tests.
//!
//! A shaded triangle's colour is linear in the same way, so each of its
//! three channels is kept as an edge is, with the same two steps, and
//! the per-pixel work is three adds more. The host works out each
//! channel's value at the box's first pixel and its steps, so those
//! need no setup here at all.
//!
//! It finds its own work. The display list is in memory, in the
//! format `crate::dl` states, and the rasteriser reads it over the
//! same link it writes pixels on: it reads the count at `CTRL` until
//! it is not zero, then the sixteen words of each instruction at `DL`,
//! as one read burst of sixteen beats, then walks what they say. The
//! count is sixteen bits, so a list holds up to 65535 entries. That
//! front end is written as the sequence it is, and a second process
//! takes the link's answers every cycle.
//!
//! It draws list after list. When a list is drawn and every write it
//! made has been answered, it writes the count back to zero and reads
//! it again, so a program waits for the zero, writes the next list and
//! then its count, and the next list is drawn. `idle` says the same on
//! a line: it falls when a count that is not zero is read and rises
//! once that list's zero has been answered.
//!
//! It reads the count only while `ring` is high. Where the count is a
//! register beside the rasteriser, as on the board (issue 985), the
//! register drives `ring` high while it is not zero, so an idle
//! rasteriser puts nothing on the link at all; where nothing does,
//! `ring` is tied high and the count is read back to back.
//!
//! It is an AXI host, and it writes as a host client writes: a run of
//! a row's pixels as one burst of up to sixteen beats, issued on
//! `issue` with its first beat on `wbeat` in the same cycle and a beat
//! a pixel after it, strobes off where a pixel is outside the primitive
//! (issue 987), with the identifier the tracker granted handed back on
//! `release` when the write response arrives. Several writes
//! are in flight, as many as the tracker has identifiers. A read's
//! identifier is handed back when its beat arrives on `rdata`.
//!
//! The framebuffer's first word is at `BASE` and a pixel is one word,
//! so a pixel's address is `BASE + ((y << LOGW) + x) * 4`.
use txhdl::comp::{
    join2, mux, until, Clock, DefaultClock, In, Mem, Out, Reg, Rx, Tx, Unit,
    Wire,
};
use txhdl::funcs::{lt_signed, sra};
use txhdl::types::{Bit, U};
use txhdl::{lower, select, with, Trace};
use txhdl_parts::bus::axi::{BurstKind, Done, Grant, Issue, R, W};

use crate::op::Kind;

/// A pixel is one word, so a pixel's byte address is its index in the
/// framebuffer shifted by this, and then offset by the framebuffer's
/// own base. A word of the display list is addressed the same way.
const WORD: usize = 2;

/// The most beats a write burst takes, less one, as AXI's `len` says
/// it: sixteen pixels (issue 987).
const RUN: u32 = 15;

/// The words of a 4 KiB page, less one, which a burst may not cross.
const PAGE: u32 = 1023;

/// Where a tiled list's entries start, in bytes past its tile table:
/// `razboj_tile::ENTRIES_AT`, room for every tile's record (issue 1255).
const ENTRIES_AT: u32 = 0x800;

/// A tile's side, and the beats of a row of it, less one, as AXI's
/// `len` says it: `razboj_tile::TILE`.
const TILE: u32 = 64;
const TILE_LEN: u32 = 63;

// Those three are razboj_tile's, which the build holds them to.
const _: () = assert!(ENTRIES_AT as usize == razboj_tile::ENTRIES_AT);
const _: () = assert!(TILE == razboj_tile::TILE && TILE_LEN == TILE - 1);

/// The shift from an instruction's index to its byte address, which
/// is `razboj::dl::BYTE_SHIFT` and is stated here because the lowering
/// wants a constant it can see.
const SHIFT: usize = 6;

// begin{state}
/// The rasteriser. `A` is the address width, `I` the AXI identifier
/// width, the screen is `1 << LOGW` by `H` pixels, and the
/// framebuffer's first word is at byte address `BASE`, which is where
/// a design puts it in whatever it writes into.
#[derive(Trace, Default)]
pub struct Raster<
    const A: usize,
    const I: usize,
    const LOGW: usize,
    const H: usize,
    const BASE: usize,
    const DL: usize,
    const CTRL: usize,
> {
    /// Which entry is being walked. A triangle is the one whose edge
    /// functions are tested, and the waveform names it.
    pub kind: Reg<Kind>,
    pub colour: Reg<U<24>>,
    /// The alpha the entry writes, in the pixel's top byte.
    pub alpha: Reg<U<8>>,
    /// Where the walk is, and the box it walks: the first column, the
    /// last column and the last row.
    pub x: Reg<U<16>>,
    pub y: Reg<U<16>>,
    pub xa: Reg<U<16>>,
    pub xb: Reg<U<16>>,
    pub yb: Reg<U<16>>,
    /// Each edge at this pixel, at the start of this row, and its two
    /// steps. Two's complement in thirty-two bits, which is wide
    /// enough for every product of two differences of vertices; the
    /// sign is the top bit.
    pub e0: Reg<U<32>>,
    pub e1: Reg<U<32>>,
    pub e2: Reg<U<32>>,
    pub r0: Reg<U<32>>,
    pub r1: Reg<U<32>>,
    pub r2: Reg<U<32>>,
    pub d0x: Reg<U<32>>,
    pub d1x: Reg<U<32>>,
    pub d2x: Reg<U<32>>,
    pub d0y: Reg<U<32>>,
    pub d1y: Reg<U<32>>,
    pub d2y: Reg<U<32>>,
    /// A shaded triangle's three channels, kept as the edges are: each
    /// at this pixel, at the start of this row, and its two steps.
    /// Sixteen bits of fraction below the byte the pixel takes.
    pub cr: Reg<U<32>>,
    pub cg: Reg<U<32>>,
    pub cb: Reg<U<32>>,
    pub lr: Reg<U<32>>,
    pub lg: Reg<U<32>>,
    pub lb: Reg<U<32>>,
    pub crx: Reg<U<32>>,
    pub cgx: Reg<U<32>>,
    pub cbx: Reg<U<32>>,
    pub cry: Reg<U<32>>,
    pub cgy: Reg<U<32>>,
    pub cby: Reg<U<32>>,
    /// Pixels written, and responses taken, each counted round; their
    /// difference is what is in flight, and the two are apart so that
    /// the walk and the answers each keep a count of their own.
    pub issued: Reg<U<4>>,
    pub answered: Reg<U<4>>,
    /// Writes issued whose response has not come back.
    pub inflight: Wire<U<4>>,
    /// Whether this pixel is in the primitive. A wire and not a
    /// `let`, so that it has a name in the trace and in the netlist:
    /// it is the one thing a waveform of a triangle wants to show,
    /// and the walk waits on it.
    pub hit: Wire<Bit>,
    /// The word this pixel takes, the entry's alpha above its colour,
    /// which for a shaded triangle is worked out from its three
    /// channels here.
    pub rgb: Wire<U<32>>,
    /// Instructions in the list, once the count has been read.
    pub left: Reg<U<16>>,
    /// Which instruction is being fetched, and which of its words.
    pub insn: Reg<U<16>>,
    pub word: Reg<U<5>>,
    /// The last list is drawn and its count written back to zero; a
    /// count read that is not zero clears it.
    pub finished: Reg<Bit>,
    /// The instruction being assembled, word by word.
    pub skind: Reg<Kind>,
    pub scol: Reg<U<24>>,
    pub sx0: Reg<U<10>>,
    pub sy0: Reg<U<10>>,
    pub sx1: Reg<U<10>>,
    pub sy1: Reg<U<10>>,
    pub sax: Reg<U<16>>,
    pub say: Reg<U<16>>,
    pub sbx: Reg<U<16>>,
    pub sby: Reg<U<16>>,
    pub scx: Reg<U<16>>,
    pub scy: Reg<U<16>>,
    /// The setup, over three cycles after the fetch: per edge, the
    /// distances from its first vertex to the box's first pixel, and
    /// then, in the same registers, those times the edge's steps.
    pub u0x: Reg<U<32>>,
    pub u0y: Reg<U<32>>,
    pub u1x: Reg<U<32>>,
    pub u1y: Reg<U<32>>,
    pub u2x: Reg<U<32>>,
    pub u2y: Reg<U<32>>,
    /// Which of the three edges is a top or a left one, for the fill
    /// rule.
    pub tl0: Reg<Bit>,
    pub tl1: Reg<Bit>,
    pub tl2: Reg<Bit>,
    /// The beats still owed to the write burst under way; nought when
    /// none is (issue 987).
    pub beats: Reg<U<8>>,
    /// Drawing in tiles (issue 1255): whether the list is a tile table,
    /// which bit 31 of the count says; the tiles still to draw, one for
    /// a flat list; the next tile's record; the entries the tile has;
    /// and the tile's top left pixel.
    pub tiled: Reg<Bit>,
    pub tiles: Reg<U<16>>,
    pub tile: Reg<U<16>>,
    pub n: Reg<U<16>>,
    pub ox: Reg<U<16>>,
    pub oy: Reg<U<16>>,
    /// The tile buffer: a tile's colour, and a mark for each pixel, at
    /// `{y, x}`, the low six bits of each coordinate. A pixel's mark is
    /// the serial of the tile that last wrote it, so a pixel was
    /// written in this tile when its mark is this tile's serial, and no
    /// mark is ever cleared one at a time: that would be a second write,
    /// and a memory written in two places is flip-flops, not a block RAM.
    /// The serial runs from one to 255. A scrub, the walk over the whole
    /// tile writing every mark nought, comes first after a reset and
    /// again each time the serial runs out, so no old mark can equal it.
    pub bank: Mem<U<32>, 4096>,
    pub mark: Mem<U<8>, 4096>,
    pub serial: Reg<U<8>>,
    pub clean: Reg<Bit>,
    pub scrub: Reg<Bit>,
    /// The write-out: the tile's rows on the screen, the row and the
    /// column going out, and the word and the mark read a cycle ahead
    /// for the next beat, so that the bank's read lands in a register,
    /// which is what makes it a block RAM.
    pub th: Reg<U<8>>,
    pub wr: Reg<U<6>>,
    pub wc: Reg<U<7>>,
    pub rd: Reg<U<32>>,
    pub rm: Reg<U<8>>,
    /// Depth (issue 992): whether the entry tests it, which only a tile
    /// does; the comparison; whether a pixel that passes writes its
    /// depth; the plane at this pixel and at the start of this row, and
    /// its two steps; and the address of the pixel under the walk in the
    /// tile, which the depth bank is read and written at.
    pub zon: Reg<Bit>,
    pub deep: Reg<Bit>,
    pub zfunc: Reg<U<3>>,
    pub zwrite: Reg<Bit>,
    pub zc: Reg<U<32>>,
    pub zr: Reg<U<32>>,
    pub zdx: Reg<U<32>>,
    pub zdy: Reg<U<32>>,
    pub pa: Reg<U<12>>,
    /// The depth bank, a tile's depths, and above each its stencil (issue
    /// 998), read and written at one address only, so that it is a block
    /// RAM of one port; a mark for each, the serial of the tile that
    /// wrote it as with the colour, so that a depth not written in this
    /// tile reads as the farthest and a stencil as nought; the word and
    /// its mark read for the pixel under the walk, and the pixel's own
    /// depth; whether the pixel passed, decided a turn after the read so
    /// that the banks' write enables come from a register; and whether
    /// the bank is written, and the word it takes. A pixel that changes
    /// one of the two writes the other back as it was, so one mark holds
    /// for both.
    pub zbank: Mem<U<24>, 4096>,
    pub zmark: Mem<U<8>, 4096>,
    pub dread: Reg<U<24>>,
    pub dtag: Reg<U<8>>,
    pub zq: Reg<U<16>>,
    pub zpass: Reg<Bit>,
    pub zsw: Reg<Bit>,
    pub zsv: Reg<U<24>>,
    /// The stencil (issue 998): whether the entry tests it, the comparison,
    /// the reference, the value mask and the write mask, and the operations
    /// for a stencil failure, a depth failure and a pass.
    pub sten: Reg<Bit>,
    pub sfunc: Reg<U<3>>,
    pub sref: Reg<U<8>>,
    pub smask: Reg<U<8>>,
    pub swmask: Reg<U<8>>,
    pub sfail: Reg<U<3>>,
    pub szfail: Reg<U<3>>,
    pub szpass: Reg<U<3>>,
    /// The pixel's state (issue 993), which only a tile has: whether the
    /// entry has any, whether it blends and with which two factors,
    /// whether it tests alpha, how and against what, and the channels it
    /// writes, a bit a byte.
    pub son: Reg<Bit>,
    pub bon: Reg<Bit>,
    pub sfac: Reg<U<4>>,
    pub dfac: Reg<U<4>>,
    pub aon: Reg<Bit>,
    pub afunc: Reg<U<3>>,
    pub aref: Reg<U<8>>,
    pub cmask: Reg<U<4>>,
    /// The logic operation (issue 998): whether it is on, in place of the
    /// blend, which of GL's sixteen, and the pixel it makes, a turn
    /// before the write.
    pub lon: Reg<Bit>,
    pub lop: Reg<U<4>>,
    pub lres: Reg<U<32>>,
    /// Fog (issue 998): whether the entry is fogged and the fog's colour;
    /// the factor's plane, at this pixel, at the start of this row, and
    /// its two steps, as `zc`, `zr`, `zdx` and `zdy` are depth's; and the
    /// pixel's colour by its factor and the fog's by what is left, a
    /// channel each, with the pixel's alpha, a turn before they are
    /// divided.
    pub fon: Reg<Bit>,
    pub fcol: Reg<U<24>>,
    pub fc: Reg<U<32>>,
    pub fr: Reg<U<32>>,
    pub fdx: Reg<U<32>>,
    pub fdy: Reg<U<32>>,
    pub fsr: Reg<U<17>>,
    pub fsg: Reg<U<17>>,
    pub fsb: Reg<U<17>>,
    pub fsa: Reg<U<8>>,
    /// A shaded triangle's alpha plane (issue 1520), in the second slot's
    /// words 12 to 15: whether it is on, and the plane at this pixel, at
    /// the start of this row, and its two steps, as the colour channels'
    /// are. With it on, the pixel's alpha is the plane's rather than the
    /// entry's one alpha.
    pub pon: Reg<Bit>,
    pub pac: Reg<U<32>>,
    pub par: Reg<U<32>>,
    pub padx: Reg<U<32>>,
    pub pady: Reg<U<32>>,
    /// The tile's colour again, written where and as the bank is and
    /// read only by the walk, for the colour already at a pixel, so that
    /// each of the two is a block RAM of one write and one read; the
    /// colour read there; the pixel's own, held a turn; and the colour
    /// the pixel writes, blended and masked.
    /// A block RAM, which Vivado makes of it only while the blend's
    /// products below keep out of DSP slices (issue 1343).
    #[ram_style("block")]
    pub dbank: Mem<U<32>, 4096>,
    pub dcol: Reg<U<32>>,
    pub srcq: Reg<U<32>>,
    pub bout: Reg<U<32>>,
    /// The blend's three turns between the read and the write, so that
    /// none holds more than one of them (issue 993): each channel's two
    /// factors, a byte each, alpha highest; then each channel's sum of
    /// its two products, before the divide.
    pub fsq: Reg<U<32>>,
    pub fdq: Reg<U<32>>,
    #[use_dsp("no")]
    pub sum_a: Reg<U<17>>,
    #[use_dsp("no")]
    pub sum_r: Reg<U<17>>,
    #[use_dsp("no")]
    pub sum_g: Reg<U<17>>,
    #[use_dsp("no")]
    pub sum_b: Reg<U<17>>,
    /// Whether the tile is loaded from the framebuffer before its
    /// entries, as one more entry after the scrub (issue 993).
    pub load: Reg<Bit>,
    /// Whether the entry is textured, which gives it two texture slots
    /// (issue 997), and whether it samples, which only a tile does.
    pub texd: Reg<Bit>,
    pub tex_on: Reg<Bit>,
    /// The planes `u q`, `v q` and `q`, each at this pixel, at the start
    /// of this row, and its two steps, as a colour channel's are; and a
    /// plane's low word, held while its high word lands.
    pub tuc: Reg<U<64>>,
    pub tur: Reg<U<64>>,
    pub tudx: Reg<U<64>>,
    pub tudy: Reg<U<64>>,
    pub tvc: Reg<U<64>>,
    pub tvr: Reg<U<64>>,
    pub tvdx: Reg<U<64>>,
    pub tvdy: Reg<U<64>>,
    pub tqc: Reg<U<64>>,
    pub tqr: Reg<U<64>>,
    pub tqdx: Reg<U<64>>,
    pub tqdy: Reg<U<64>>,
    pub tlo: Reg<U<32>>,
    /// The texture: its descriptor's address, its environment and the
    /// environment's colour, and from the descriptor its base level's
    /// sides, its levels, its two filters, its two wrap modes, how its
    /// texels are read, and each level's address.
    pub tdesc: Reg<U<32>>,
    pub tenv: Reg<U<3>>,
    pub tenvc: Reg<U<32>>,
    pub tlogw: Reg<U<4>>,
    pub tlogh: Reg<U<4>>,
    pub tlevels: Reg<U<4>>,
    pub tmin: Reg<U<3>>,
    pub tmag: Reg<Bit>,
    pub tcs: Reg<Bit>,
    pub tct: Reg<Bit>,
    pub tclass: Reg<U<3>>,
    pub tlvl: Mem<U<32>, 16>,
    /// The level of detail's numerators (issue 997), as a channel's
    /// planes: the two that step down at this row, the two that step
    /// across at this pixel and at the start of the row, their steps,
    /// and the shift they were taken by.
    pub tlodk: Reg<U<8>>,
    pub tnux: Reg<U<32>>,
    pub tnuxd: Reg<U<32>>,
    pub tnvx: Reg<U<32>>,
    pub tnvxd: Reg<U<32>>,
    pub tnuy: Reg<U<32>>,
    pub tnuy0: Reg<U<32>>,
    pub tnuyd: Reg<U<32>>,
    pub tnvy: Reg<U<32>>,
    pub tnvy0: Reg<U<32>>,
    pub tnvyd: Reg<U<32>>,
    /// A textured pixel's turns, as `tex::texel_uv` has them: `q`'s
    /// leading zeros and its top 32 bits from its leading one; the
    /// reciprocal's first guess, its Newton step's error, and the
    /// reciprocal; `u q` and `v q` times it; the texel coordinates, with
    /// eight bits of fraction; the texel's address; and what the cache
    /// holds for it.
    pub tn: Reg<U<7>>,
    pub tx: Reg<U<32>>,
    pub tr0: Reg<U<17>>,
    pub te: Reg<U<50>>,
    pub trc: Reg<U<28>>,
    pub tpu: Reg<U<96>>,
    pub tpv: Reg<U<96>>,
    /// Each plane's two halves times the reciprocal, low and high,
    /// before they are added.
    pub tpul: Reg<U<64>>,
    pub tpuh: Reg<U<64>>,
    pub tpvl: Reg<U<64>>,
    pub tpvh: Reg<U<64>>,
    pub tiu: Reg<U<32>>,
    pub tiv: Reg<U<32>>,
    pub taddr: Reg<U<32>>,
    pub ttag: Reg<U<20>>,
    pub tok: Reg<Bit>,
    pub tdat: Reg<U<32>>,
    /// The texel's word in the cache, an address of its own, so that
    /// the cache's read has a register for one and nothing else; the
    /// texel's word as a refill passes, and whether it came that way,
    /// so that the read's register, `tdat`, takes nothing but the read.
    pub tca: Reg<U<10>>,
    pub tfil: Reg<U<32>>,
    pub tmiss: Reg<Bit>,
    /// The environment's four products, before their divide, and the
    /// textured pixel's colour. The products are made in logic and not
    /// in DSP slices: a slice would take the cache's read register into
    /// its input pipeline, which leaves the read without one, and the
    /// words could not be a block RAM (#1343).
    #[use_dsp("no")]
    pub tma: Reg<U<17>>,
    #[use_dsp("no")]
    pub tmr: Reg<U<17>>,
    #[use_dsp("no")]
    pub tmg: Reg<U<17>>,
    #[use_dsp("no")]
    pub tmb: Reg<U<17>>,
    /// `BLEND`'s and `DECAL`'s sums before their divide, three colour
    /// channels each: the fragment and the environment's colour by the
    /// texel, and the fragment and the texel by the texel's alpha.
    pub mbl: Reg<U<51>>,
    pub mdc: Reg<U<51>>,
    pub tcol: Reg<U<32>>,
    /// The level of detail (issue 997), as `tex::lod` has it: the
    /// largest numerator's magnitude, its leading zeros and its top 32
    /// bits from its leading one; the lines' products for its `log2`
    /// and for `q`'s; and the level of detail, in 8.8.
    pub mlm: Reg<U<32>>,
    pub mlz: Reg<U<7>>,
    pub mlx: Reg<U<32>>,
    /// The two products are made in logic: in a DSP slice Vivado took `tx`
    /// into the slice's input register, which left `q`'s normalisation and
    /// the product's input in one cycle at 0.064 ns of slack (#1475).
    #[use_dsp("no")]
    pub mpq: Reg<U<22>>,
    #[use_dsp("no")]
    pub mpm: Reg<U<22>>,
    pub mlod: Reg<U<32>>,
    /// What `tex::sample` makes of it: the level sampled, and the next
    /// when two are blended; whether two are; whether a level is
    /// filtered linearly; the fraction two are blended by; and how many
    /// texels the pixel reads, one or four a level.
    pub mlev1: Reg<U<4>>,
    pub mlev2: Reg<U<4>>,
    pub mtwo: Reg<Bit>,
    pub mlin: Reg<Bit>,
    pub mfr: Reg<U<8>>,
    pub mnt: Reg<U<4>>,
    /// A texel's turns: which of the pixel's it is, its level, its pass
    /// and its place among the four; the coordinates at its level and
    /// that level's sides and address; the texel's column and row,
    /// wrapped; its weight; the weight times each channel; and each
    /// pass's sums of those.
    pub mk: Reg<U<4>>,
    pub mpass: Reg<Bit>,
    pub mt: Reg<U<2>>,
    pub mus: Reg<U<32>>,
    pub mvs: Reg<U<32>>,
    pub mlw: Reg<U<4>>,
    pub mlh: Reg<U<4>>,
    pub mbase: Reg<U<32>>,
    pub mii: Reg<U<32>>,
    pub mjj: Reg<U<32>>,
    pub mw: Reg<U<17>>,
    /// The weight times each channel, which takes the cache's word, so
    /// in logic and not in DSP slices (#1343); and each pass's sums of
    /// those, 25 bits a channel, alpha highest.
    #[use_dsp("no")]
    pub mpp: Reg<U<100>>,
    pub msum1: Reg<U<100>>,
    pub msum2: Reg<U<100>>,
    /// Each pass's sample, the two levels' blend before its shift, 17
    /// bits a channel, and the texel the environment takes.
    pub ms1: Reg<U<32>>,
    pub ms2: Reg<U<32>>,
    pub mkk: Reg<U<68>>,
    pub mtx: Reg<U<32>>,
    /// The texture cache: 64 lines, each a block of four by four texels,
    /// one burst of 64 bytes, direct mapped by the block's address. The
    /// words, a block RAM of one write, the refill's, and one read, the
    /// pixel's; each line's tag, the address above the line's index; and
    /// a bit a line saying it holds anything, all cleared when a list
    /// starts, since a program may change its textures between lists.
    #[ram_style("block")]
    pub cdata: Mem<U<32>, 1024>,
    pub ctag: Mem<U<20>, 64>,
    pub cvalid: Reg<U<64>>,
}
// end{state}

/// Whether an edge is a top or a left one, from how its function moves:
/// by `dx` a pixel to the right and `dy` a pixel down. It is a left edge
/// when the function grows to the right, which is where inside lies, and
/// a top edge when it is level and grows downwards. A pixel centre on
/// such an edge is drawn, and one on any other edge is not, so that of
/// two triangles sharing an edge exactly one draws a pixel on it.
#[lower]
fn top_left(dx: U<32>, dy: U<32>) -> Bit {
    let right = !dx.bit(31) & Bit::from(dx != 0);
    let down = !dy.bit(31) & Bit::from(dy != 0);
    right | (Bit::from(dx == 0) & down)
}

/// One channel of a shaded pixel: the byte above a channel's sixteen
/// bits of fraction, nought when the value is below nought and 255 when
/// it is 256 or more, which a pixel on the box's edge can reach since
/// the plane goes on past the triangle.
#[lower]
fn channel(v: U<32>) -> U<8> {
    let over = Bit::from(v.slice::<24, 8>() != 0);
    let top = mux(over, U::<8>::from(255u8), v.slice::<16, 8>());
    mux(v.bit(31), U::<8>::from(0u8), top)
}

/// A pixel's depth from its plane (issue 992): the sixteen bits above
/// the plane's twelve of fraction, nought when the value is below nought
/// and the farthest, `0xffff`, when it is past it, as `model::depth` has
/// it.
#[lower]
fn depth16(v: U<32>) -> U<16> {
    let over = Bit::from(v.slice::<28, 3>() != 0);
    let top = mux(over, U::<16>::from(0xffffu32), v.slice::<12, 16>());
    mux(v.bit(31), U::<16>::from(0u8), top)
}

/// The stencil `s` after GL ES 1.1's operation `op` (issue 998), with the
/// reference `r`, as `op::stencil::op` has it: kept, nought, the
/// reference, one more or one less held to a byte, or turned over.
#[lower]
fn stencil_step(op: U<3>, s: U<8>, r: U<8>) -> U<8> {
    let (z, top) = (U::<8>::from(0u8), U::<8>::from(255u8));
    let inc = mux(Bit::from(s == top), top, s + U::<8>::from(1u8));
    let dec = mux(Bit::from(s == z), z, s - U::<8>::from(1u8));
    let one = Bit::from(op == U::<3>::from(1u8));
    let two = Bit::from(op == U::<3>::from(2u8));
    let three = Bit::from(op == U::<3>::from(3u8));
    let four = Bit::from(op == U::<3>::from(4u8));
    let five = Bit::from(op == U::<3>::from(5u8));
    let flip = mux(five, s ^ top, s);
    mux(one, z, mux(two, r, mux(three, inc, mux(four, dec, flip))))
}

/// The stencil `s` written after the operation `op` with the reference
/// `r`: the operation's bits where the write mask `wm` holds them and
/// `s`'s elsewhere, or `s` whole when the stencil test is off (`on`
/// clear).
#[lower]
fn stencil_put(op: U<3>, s: U<8>, r: U<8>, on: Bit, wm: U<8>) -> U<8> {
    let sn = stencil_step(op, s, r);
    mux(on, (s & !wm) | (sn & wm), s)
}

/// Whether a pixel at depth `z` passes `func`, GL's comparisons from
/// `GL_NEVER` to `GL_ALWAYS` in GL's order, against the depth `d` there,
/// as `model::passes` has it.
#[lower]
fn depth_pass(func: U<3>, z: U<16>, d: U<16>) -> Bit {
    let lt = Bit::from(z < d);
    let eq = Bit::from(z == d);
    let gt = Bit::from(z > d);
    let hi = mux(
        func.bit(1),
        mux(func.bit(0), Bit::One, gt | eq),
        mux(func.bit(0), !eq, gt),
    );
    let lo = mux(
        func.bit(1),
        mux(func.bit(0), lt | eq, eq),
        mux(func.bit(0), lt, Bit::Zero),
    );
    mux(func.bit(2), hi, lo)
}

/// One blend factor (issue 993), as `model::factor` has it: `f`, four
/// bits from `GL_ZERO` to `GL_SRC_ALPHA_SATURATE`, of a channel whose
/// source byte is `s` and destination byte `d`, the source's alpha being
/// `sa` and the destination's `da`, and `alpha` saying the channel is
/// alpha itself. The factors come in pairs, a value and 255 less it, so
/// bits 3 to 1 pick the value and bit 0 says which of the pair. Codes
/// past ten are not defined.
#[lower]
fn factor(f: U<4>, s: U<8>, d: U<8>, sa: U<8>, da: U<8>, alpha: Bit) -> U<8> {
    let full = U::<8>::from(255u32);
    let room = full - da;
    let sat = mux(alpha, full, mux(sa < room, sa, room));
    let low = mux(f.bit(1), s, U::<8>::from(0u8));
    let mid = mux(f.bit(1), da, sa);
    let high = mux(f.bit(1), sat, d);
    let v = mux(f.bit(3), high, mux(f.bit(2), mid, low));
    mux(f.bit(0), full - v, v)
}

/// One channel's blend before its divide (issue 993): `s Fs + d Fd`,
/// seventeen bits.
#[lower]
fn blend_sum(s: U<8>, d: U<8>, fs: U<8>, fd: U<8>) -> U<17> {
    s.resize::<17>().mul::<17>(fs.resize::<17>())
        + d.resize::<17>().mul::<17>(fd.resize::<17>())
}

/// That sum over 255, rounded to the nearest without a divider, and at
/// most 255, as `model::blend` does it.
#[lower]
fn over255(x: U<17>) -> U<8> {
    let y = x + U::<17>::from(128u32);
    let r = (y + (y >> 8usize)) >> 8usize;
    mux(
        r > U::<17>::from(255u32),
        U::<8>::from(255u32),
        r.slice::<0, 8>(),
    )
}

/// GL's logic operation `op` of the pixel `s` and the colour there `d`,
/// bit by bit (issue 998), as `op::logic` has it: bit `3 - (2s + d)` of
/// the operation is the result for a source bit `s` and a destination
/// bit `d`.
#[lower]
fn logic_op(op: U<4>, s: U<32>, d: U<32>) -> U<32> {
    let z = U::<32>::from(0u8);
    let all = U::<32>::from(u32::MAX);
    let (ns, nd) = (s ^ all, d ^ all);
    let t3 = mux(op.bit(3), all, z);
    let t2 = mux(op.bit(2), all, z);
    let t1 = mux(op.bit(1), all, z);
    let t0 = mux(op.bit(0), all, z);
    (t3 & ns & nd) | (t2 & ns & d) | (t1 & s & nd) | (t0 & s & d)
}

/// The bytes of `new` that `mask` holds, a bit a byte, over `old`.
#[lower]
fn masked(new: U<32>, old: U<32>, mask: U<4>) -> U<32> {
    let b = mux(mask.bit(0), new.slice::<0, 8>(), old.slice::<0, 8>());
    let g = mux(mask.bit(1), new.slice::<8, 8>(), old.slice::<8, 8>());
    let r = mux(mask.bit(2), new.slice::<16, 8>(), old.slice::<16, 8>());
    let a = mux(mask.bit(3), new.slice::<24, 8>(), old.slice::<24, 8>());
    a.concat::<8, 16>(r).concat::<8, 24>(g).concat::<8, 32>(b)
}

/// The reciprocal's first guess (issue 997) follows a line in each of 32
/// segments of a mantissa between one and two: this is the line's value
/// at the start of the segment `k`, `2^17 / m` with 16 bits of fraction,
/// and [`seed_fall`] how far it falls over the segment. `tex::seed_line`
/// is the formula, which a test holds both to.
#[lower]
pub(crate) fn seed_start(k: U<5>) -> U<17> {
    select!(k.raw() => {
        0 => U::<17>::from(131057u32),
        1 => U::<17>::from(127086u32),
        2 => U::<17>::from(123349u32),
        3 => U::<17>::from(119826u32),
        4 => U::<17>::from(116498u32),
        5 => U::<17>::from(113350u32),
        6 => U::<17>::from(110367u32),
        7 => U::<17>::from(107538u32),
        8 => U::<17>::from(104850u32),
        9 => U::<17>::from(102293u32),
        10 => U::<17>::from(99858u32),
        11 => U::<17>::from(97536u32),
        12 => U::<17>::from(95319u32),
        13 => U::<17>::from(93201u32),
        14 => U::<17>::from(91175u32),
        15 => U::<17>::from(89236u32),
        16 => U::<17>::from(87377u32),
        17 => U::<17>::from(85594u32),
        18 => U::<17>::from(83882u32),
        19 => U::<17>::from(82237u32),
        20 => U::<17>::from(80656u32),
        21 => U::<17>::from(79134u32),
        22 => U::<17>::from(77669u32),
        23 => U::<17>::from(76257u32),
        24 => U::<17>::from(74895u32),
        25 => U::<17>::from(73582u32),
        26 => U::<17>::from(72313u32),
        27 => U::<17>::from(71087u32),
        28 => U::<17>::from(69903u32),
        29 => U::<17>::from(68757u32),
        30 => U::<17>::from(67648u32),
        _ => U::<17>::from(66574u32),
    })
}

/// The fall of the segment `k`'s line over its 1024 steps, in 1024ths of
/// a step: see [`seed_start`].
#[lower]
pub(crate) fn seed_fall(k: U<5>) -> U<12> {
    select!(k.raw() => {
        0 => U::<12>::from(3972u32),
        1 => U::<12>::from(3738u32),
        2 => U::<12>::from(3525u32),
        3 => U::<12>::from(3329u32),
        4 => U::<12>::from(3149u32),
        5 => U::<12>::from(2983u32),
        6 => U::<12>::from(2830u32),
        7 => U::<12>::from(2689u32),
        8 => U::<12>::from(2558u32),
        9 => U::<12>::from(2436u32),
        10 => U::<12>::from(2322u32),
        11 => U::<12>::from(2217u32),
        12 => U::<12>::from(2118u32),
        13 => U::<12>::from(2026u32),
        14 => U::<12>::from(1940u32),
        15 => U::<12>::from(1859u32),
        16 => U::<12>::from(1783u32),
        17 => U::<12>::from(1712u32),
        18 => U::<12>::from(1645u32),
        19 => U::<12>::from(1582u32),
        20 => U::<12>::from(1522u32),
        21 => U::<12>::from(1466u32),
        22 => U::<12>::from(1412u32),
        23 => U::<12>::from(1362u32),
        24 => U::<12>::from(1314u32),
        25 => U::<12>::from(1269u32),
        26 => U::<12>::from(1226u32),
        27 => U::<12>::from(1185u32),
        28 => U::<12>::from(1146u32),
        29 => U::<12>::from(1109u32),
        30 => U::<12>::from(1074u32),
        _ => U::<12>::from(1040u32),
    })
}

/// The fraction of `log2` (#997), for the level of detail, follows a
/// line in each of 32 segments of a mantissa between one and two: this
/// is the line's value at the start of the segment `k`, `65536 log2 m`,
/// and [`log_rise`] how far it rises over the segment. `tex::log_line`
/// is the formula, which a test holds both to.
#[lower]
pub(crate) fn log_start(k: U<5>) -> U<16> {
    select!(k.raw() => {
        0 => U::<16>::from(6u32),
        1 => U::<16>::from(2915u32),
        2 => U::<16>::from(5737u32),
        3 => U::<16>::from(8477u32),
        4 => U::<16>::from(11141u32),
        5 => U::<16>::from(13731u32),
        6 => U::<16>::from(16252u32),
        7 => U::<16>::from(18708u32),
        8 => U::<16>::from(21101u32),
        9 => U::<16>::from(23436u32),
        10 => U::<16>::from(25714u32),
        11 => U::<16>::from(27939u32),
        12 => U::<16>::from(30112u32),
        13 => U::<16>::from(32237u32),
        14 => U::<16>::from(34315u32),
        15 => U::<16>::from(36348u32),
        16 => U::<16>::from(38339u32),
        17 => U::<16>::from(40288u32),
        18 => U::<16>::from(42198u32),
        19 => U::<16>::from(44070u32),
        20 => U::<16>::from(45906u32),
        21 => U::<16>::from(47707u32),
        22 => U::<16>::from(49474u32),
        23 => U::<16>::from(51209u32),
        24 => U::<16>::from(52913u32),
        25 => U::<16>::from(54586u32),
        26 => U::<16>::from(56230u32),
        27 => U::<16>::from(57847u32),
        28 => U::<16>::from(59436u32),
        29 => U::<16>::from(60998u32),
        30 => U::<16>::from(62536u32),
        _ => U::<16>::from(64048u32),
    })
}

/// The rise of the segment `k`'s line over its 1024 steps: see
/// [`log_start`].
#[lower]
pub(crate) fn log_rise(k: U<5>) -> U<12> {
    select!(k.raw() => {
        0 => U::<12>::from(2909u32),
        1 => U::<12>::from(2823u32),
        2 => U::<12>::from(2741u32),
        3 => U::<12>::from(2664u32),
        4 => U::<12>::from(2591u32),
        5 => U::<12>::from(2521u32),
        6 => U::<12>::from(2456u32),
        7 => U::<12>::from(2394u32),
        8 => U::<12>::from(2335u32),
        9 => U::<12>::from(2278u32),
        10 => U::<12>::from(2225u32),
        11 => U::<12>::from(2174u32),
        12 => U::<12>::from(2125u32),
        13 => U::<12>::from(2078u32),
        14 => U::<12>::from(2033u32),
        15 => U::<12>::from(1991u32),
        16 => U::<12>::from(1950u32),
        17 => U::<12>::from(1910u32),
        18 => U::<12>::from(1872u32),
        19 => U::<12>::from(1836u32),
        20 => U::<12>::from(1801u32),
        21 => U::<12>::from(1767u32),
        22 => U::<12>::from(1735u32),
        23 => U::<12>::from(1704u32),
        24 => U::<12>::from(1673u32),
        25 => U::<12>::from(1644u32),
        26 => U::<12>::from(1616u32),
        27 => U::<12>::from(1589u32),
        28 => U::<12>::from(1563u32),
        29 => U::<12>::from(1537u32),
        30 => U::<12>::from(1513u32),
        _ => U::<12>::from(1489u32),
    })
}

/// The leading zeros of `v`, which is not nought: how far `tex::normal`
/// shifts `q` (issue 997), found a half at a time.
#[lower]
fn lz64(v: U<64>) -> U<7> {
    let z5 = Bit::from(v.slice::<32, 32>() == 0);
    let a = mux(z5, v << 32usize, v);
    let z4 = Bit::from(a.slice::<48, 16>() == 0);
    let b = mux(z4, a << 16usize, a);
    let z3 = Bit::from(b.slice::<56, 8>() == 0);
    let c = mux(z3, b << 8usize, b);
    let z2 = Bit::from(c.slice::<60, 4>() == 0);
    let d = mux(z2, c << 4usize, c);
    let z1 = Bit::from(d.slice::<62, 2>() == 0);
    let e = mux(z1, d << 2usize, d);
    let z0 = !e.bit(63);
    let none = U::<7>::from(0u8);
    mux(z5, U::<7>::from(32u8), none)
        + mux(z4, U::<7>::from(16u8), none)
        + mux(z3, U::<7>::from(8u8), none)
        + mux(z2, U::<7>::from(4u8), none)
        + mux(z1, U::<7>::from(2u8), none)
        + mux(z0, U::<7>::from(1u8), none)
}

/// `v`, which is not nought, shifted up until its top bit is one, as
/// [`lz64`] counts the shift.
#[lower]
fn norm64(v: U<64>) -> U<64> {
    let a = mux(Bit::from(v.slice::<32, 32>() == 0), v << 32usize, v);
    let b = mux(Bit::from(a.slice::<48, 16>() == 0), a << 16usize, a);
    let c = mux(Bit::from(b.slice::<56, 8>() == 0), b << 8usize, b);
    let d = mux(Bit::from(c.slice::<60, 4>() == 0), c << 4usize, c);
    let e = mux(Bit::from(d.slice::<62, 2>() == 0), d << 2usize, d);
    mux(!e.bit(63), e << 1usize, e)
}

/// A texel coordinate in two's complement, kept within `2^30` either way
/// and cut to its low 32 bits, as `tex::texel_uv` keeps it.
#[lower]
fn clamp30(v: U<96>) -> U<32> {
    let hi = U::<96>::from(1u64 << 30);
    let lo = U::<96>::from(0u8) - hi;
    let low = lt_signed(v, lo);
    let high = lt_signed(hi, v);
    mux(low, lo, mux(high, hi, v)).slice::<0, 32>()
}

/// A texel's column or row `i`, in two's complement, onto a side of
/// `2^log` texels: clamped to the edge, or repeated, as `tex::wrap` has
/// it.
#[lower]
fn wrap(i: U<32>, log: U<4>, clamp: Bit) -> U<32> {
    let side = U::<32>::from(1u8) << (log.raw() as usize);
    let last = side - U::<32>::from(1u8);
    let under = lt_signed(i, U::<32>::from(0u8));
    let past = !lt_signed(i, side);
    let held = mux(under, U::<32>::from(0u8), mux(past, last, i));
    mux(clamp, held, i & last)
}

/// The byte offset of the texel `(i, j)` in a base level `2^log_w`
/// texels wide, in its blocks of four by four, as
/// `razboj_tile::tex::texel_offset` has it.
#[lower]
fn texel_at(i: U<32>, j: U<32>, log_w: U<4>) -> U<32> {
    let two = U::<4>::from(2u8);
    let across = mux(log_w < two, U::<4>::from(0u8), log_w - two);
    let block = ((j >> 2usize) << (across.raw() as usize)) + (i >> 2usize);
    let three = U::<32>::from(3u8);
    let within = ((j & three) << 2usize) + (i & three);
    (block << 6usize) + (within << 2usize)
}

/// The largest magnitude of four numerators in two's complement, as
/// `tex::lod` takes it (issue 997).
#[lower]
fn biggest(a: U<32>, b: U<32>, c: U<32>, d: U<32>) -> U<32> {
    let z = U::<32>::from(0u8);
    let ma = mux(a.bit(31), z - a, a);
    let mb = mux(b.bit(31), z - b, b);
    let mc = mux(c.bit(31), z - c, c);
    let md = mux(d.bit(31), z - d, d);
    // The six comparisons side by side and one choice after them,
    // rather than the two pairs' larger and then the larger of those,
    // which put three comparisons one after another: a path of eighteen
    // levels on the flagship (#1565). The largest is the same value
    // whichever of two equal ones is taken.
    let (ab, ac, ad) = (ma < mb, ma < mc, ma < md);
    let (bc, bd, cd) = (mb < mc, mb < md, mc < md);
    let a_top = !ab & !ac & !ad;
    let b_top = !bc & !bd;
    mux(a_top, ma, mux(b_top, mb, mux(cd, md, mc)))
}

/// A pixel's level of detail in 8.8, as `tex::lod` has it: `log2` of
/// the largest numerator, from its leading zeros `lz` and its line's
/// fraction `fm`, plus the numerators' shift `k`, less the 80 bits of
/// the planes' units and twice `log2 q`, from `q`'s leading zeros `n`
/// and its fraction `fq`; and below every level when every numerator is
/// nought.
#[lower]
fn lod_of(
    lz: U<7>,
    fm: U<32>,
    k: U<8>,
    n: U<7>,
    fq: U<32>,
    none: Bit,
) -> U<32> {
    let m = ((U::<32>::from(31u8) - lz.resize::<32>()) << 8usize) + fm;
    let q = ((U::<32>::from(15u8) - n.resize::<32>()) << 8usize) + fq;
    let s = (k.resize::<32>() << 8usize) - U::<32>::from(80u32 << 8);
    let low = U::<32>::from(0u8) - U::<32>::from(1u32 << 20);
    mux(none, low, m + s - (q << 1usize))
}

/// What `tex::sample` reads for the level of detail `lod` under the
/// filters `min` and `mag` of a texture of `levels`: bits 3 to 0 the
/// level, 7 to 4 the next, 8 whether the two are blended, 9 whether a
/// level is filtered linearly, 17 to 10 the fraction they are blended
/// by, and 21 to 18 how many texels that is, one or four a level.
#[lower]
fn pick_level(lod: U<32>, min: U<3>, mag: Bit, levels: U<4>) -> U<22> {
    let z = U::<32>::from(0u8);
    let one = U::<32>::from(1u8);
    let half = U::<32>::from(128u8);
    let nmn = Bit::from(min == 2);
    let nml = Bit::from(min == 4);
    // At `c` or below the texture is magnified.
    let c = mux(mag & (nmn | nml), half, z);
    let magnify = !lt_signed(c, lod);
    let top4 = mux(levels == 0, U::<4>::from(0u8), levels - U::<4>::from(1u8));
    let top = top4.resize::<32>();
    // The `_MIPMAP_NEAREST` filters: GL's `ceil(λ + 1/2) - 1`.
    let up = ((lod + U::<32>::from(383u32)) >> 8usize) - one;
    let ln = mux(lt_signed(half, lod), up, z);
    let near = mux(top < ln, top, ln);
    // The `_MIPMAP_LINEAR` filters: `λ`'s whole part and the next.
    let whole = lod >> 8usize;
    let two = Bit::from(whole < top);
    let l1 = mux(two, whole, top);
    let flat = Bit::from(min < 2);
    let nearest = Bit::from(min < 4) & !flat;
    let mip_lin = mux(nearest, Bit::from(min == 3), !nml);
    let min_lin = mux(flat, Bit::from(min == 1), mip_lin);
    let lin = mux(magnify, mag, min_lin);
    let lev = mux(magnify | flat, z, mux(nearest, near, l1));
    let twice = !magnify & !flat & !nearest & two;
    let passes = mux(twice, U::<4>::from(2u8), U::<4>::from(1u8));
    let n = mux(lin, passes << 2usize, passes);
    let next = (l1 + one).slice::<0, 4>();
    n.concat::<8, 12>(lod.slice::<0, 8>())
        .concat::<1, 13>(lin.zext::<1>())
        .concat::<1, 14>(twice.zext::<1>())
        .concat::<4, 18>(next)
        .concat::<4, 22>(lev.slice::<0, 4>())
}

/// `ADD`'s channel: the fragment's and the texel's, at most 255.
#[lower]
fn add255(f: U<8>, t: U<8>) -> U<8> {
    let s = f.resize::<9>() + t.resize::<9>();
    mux(s.bit(8), U::<8>::from(255u8), s.slice::<0, 8>())
}

/// A texel's weight times each of its four channels, 25 bits a channel,
/// alpha highest: a linear filter's terms (issue 997).
#[lower]
fn weigh(w: U<17>, t: U<32>) -> U<100> {
    let w25 = w.resize::<25>();
    let a = w25.mul::<25>(t.slice::<24, 8>().resize::<25>());
    let r = w25.mul::<25>(t.slice::<16, 8>().resize::<25>());
    let g = w25.mul::<25>(t.slice::<8, 8>().resize::<25>());
    let b = w25.mul::<25>(t.slice::<0, 8>().resize::<25>());
    a.concat::<25, 50>(r)
        .concat::<25, 75>(g)
        .concat::<25, 100>(b)
}

/// The sums of a filter's terms, a channel each: `p` added to `acc`, or
/// to nought for a level's first texel.
#[lower]
fn add_terms(first: Bit, acc: U<100>, p: U<100>) -> U<100> {
    let s = mux(first, U::<100>::from(0u8), acc);
    let a = s.slice::<75, 25>() + p.slice::<75, 25>();
    let r = s.slice::<50, 25>() + p.slice::<50, 25>();
    let g = s.slice::<25, 25>() + p.slice::<25, 25>();
    let b = s.slice::<0, 25>() + p.slice::<0, 25>();
    a.concat::<25, 50>(r)
        .concat::<25, 75>(g)
        .concat::<25, 100>(b)
}

/// A level's sample from its sums, each over `2^16`, rounded, as
/// `tex::sample_level` has it.
#[lower]
fn sum_texel(acc: U<100>) -> U<32> {
    let h = U::<25>::from(1u32 << 15);
    let a = ((acc.slice::<75, 25>() + h) >> 16usize).slice::<0, 8>();
    let r = ((acc.slice::<50, 25>() + h) >> 16usize).slice::<0, 8>();
    let g = ((acc.slice::<25, 25>() + h) >> 16usize).slice::<0, 8>();
    let b = ((acc.slice::<0, 25>() + h) >> 16usize).slice::<0, 8>();
    a.concat::<8, 16>(r).concat::<8, 24>(g).concat::<8, 32>(b)
}

/// Two levels' samples `s1` and `s2` blended by `fr`, a channel each, 17
/// bits before the shift: `(256 - fr) s1 + fr s2`.
#[lower]
fn mix_levels(s1: U<32>, s2: U<32>, fr: U<8>) -> U<68> {
    let f = fr.resize::<17>();
    let g = U::<17>::from(256u32) - f;
    let a = s1.slice::<24, 8>().resize::<17>().mul::<17>(g)
        + s2.slice::<24, 8>().resize::<17>().mul::<17>(f);
    let r = s1.slice::<16, 8>().resize::<17>().mul::<17>(g)
        + s2.slice::<16, 8>().resize::<17>().mul::<17>(f);
    let gr = s1.slice::<8, 8>().resize::<17>().mul::<17>(g)
        + s2.slice::<8, 8>().resize::<17>().mul::<17>(f);
    let b = s1.slice::<0, 8>().resize::<17>().mul::<17>(g)
        + s2.slice::<0, 8>().resize::<17>().mul::<17>(f);
    a.concat::<17, 34>(r)
        .concat::<17, 51>(gr)
        .concat::<17, 68>(b)
}

/// That blend shifted back, rounded, as `tex::sample` has it.
#[lower]
fn mixed_texel(m: U<68>) -> U<32> {
    let h = U::<17>::from(128u8);
    let a = ((m.slice::<51, 17>() + h) >> 8usize).slice::<0, 8>();
    let r = ((m.slice::<34, 17>() + h) >> 8usize).slice::<0, 8>();
    let g = ((m.slice::<17, 17>() + h) >> 8usize).slice::<0, 8>();
    let b = ((m.slice::<0, 17>() + h) >> 8usize).slice::<0, 8>();
    a.concat::<8, 16>(r).concat::<8, 24>(g).concat::<8, 32>(b)
}

/// Three colour channels' sums `x u + y v` before their divide, 17 bits
/// each: `BLEND`'s with `x` the fragment, `y` the environment's colour,
/// `v` the texel and `u` 255 less it; and `DECAL`'s with `y` the texel,
/// `v` the texel's alpha in each channel and `u` 255 less that.
#[lower]
fn sums3(x: U<32>, y: U<32>, u: U<32>, v: U<32>) -> U<51> {
    let xr = x.slice::<16, 8>().resize::<17>();
    let xg = x.slice::<8, 8>().resize::<17>();
    let xb = x.slice::<0, 8>().resize::<17>();
    let yr = y.slice::<16, 8>().resize::<17>();
    let yg = y.slice::<8, 8>().resize::<17>();
    let yb = y.slice::<0, 8>().resize::<17>();
    let ur = u.slice::<16, 8>().resize::<17>();
    let ug = u.slice::<8, 8>().resize::<17>();
    let ub = u.slice::<0, 8>().resize::<17>();
    let vr = v.slice::<16, 8>().resize::<17>();
    let vg = v.slice::<8, 8>().resize::<17>();
    let vb = v.slice::<0, 8>().resize::<17>();
    let r = xr.mul::<17>(ur) + yr.mul::<17>(vr);
    let g = xg.mul::<17>(ug) + yg.mul::<17>(vg);
    let b = xb.mul::<17>(ub) + yb.mul::<17>(vb);
    r.concat::<17, 34>(g).concat::<17, 51>(b)
}

/// Those three sums each over 255, rounded and at most 255, as
/// [`over255`] has it, in a word with nought for alpha.
#[lower]
fn over255x3(s: U<51>) -> U<32> {
    let h = U::<17>::from(128u32);
    let full = U::<17>::from(255u32);
    let yr = s.slice::<34, 17>() + h;
    let yg = s.slice::<17, 17>() + h;
    let yb = s.slice::<0, 17>() + h;
    let rr = (yr + (yr >> 8usize)) >> 8usize;
    let rg = (yg + (yg >> 8usize)) >> 8usize;
    let rb = (yb + (yb >> 8usize)) >> 8usize;
    let r = mux(full < rr, full, rr).slice::<0, 8>();
    let g = mux(full < rg, full, rg).slice::<0, 8>();
    let b = mux(full < rb, full, rb).slice::<0, 8>();
    U::<8>::from(0u8)
        .concat::<8, 16>(r)
        .concat::<8, 24>(g)
        .concat::<8, 32>(b)
}

/// A textured pixel's colour (issue 997): the fragment's `f` through the
/// environment with the texel `t`, as `tex::env` has it. `how` is the
/// texel's class, which says which of its channels it has, above the
/// environment, three bits each. `m` is the four products of the
/// fragment and the texel over 255; `l` the colour `BLEND` makes, the
/// fragment and the environment's colour by the texel; `d` the one
/// `DECAL` makes, the fragment and the texel by the texel's alpha; and
/// `a` the one `ADD` makes.
#[lower]
fn tex_env(
    f: U<32>,
    t: U<32>,
    m: U<32>,
    l: U<32>,
    d: U<32>,
    a: U<32>,
    how: U<6>,
) -> U<32> {
    let env = how.slice::<0, 3>();
    let class = how.slice::<3, 3>();
    let replace = Bit::from(env == 0);
    let modulate = Bit::from(env == 1);
    let decal = Bit::from(env == 2);
    let blend = Bit::from(env == 3);
    let alpha_only = Bit::from(class == 2);
    let lum = Bit::from(class == 3);
    let colour_only = Bit::from(class == 1) | lum;
    let la = Bit::from(class == 4);
    let f24 = f.slice::<0, 24>();
    let t24 = t.slice::<0, 24>();
    let rest = mux(blend, l.slice::<0, 24>(), a.slice::<0, 24>());
    let made = mux(decal, d.slice::<0, 24>(), rest);
    let picked = mux(replace, t24, mux(modulate, m.slice::<0, 24>(), made));
    // A class with no colour, or `DECAL` where GL leaves it undefined,
    // passes the fragment's through.
    let lum_decal = mux(lum, f24, t24);
    let shown = mux(colour_only & decal, lum_decal, picked);
    let rgb = mux(alpha_only | (la & decal), f24, shown);
    // A class with no alpha passes the fragment's through.
    let ta = t.slice::<24, 8>();
    let fa = f.slice::<24, 8>();
    let some = mux(replace, ta, mux(decal, fa, m.slice::<24, 8>()));
    mux(colour_only, fa, some).concat::<24, 32>(rgb)
}
/// Whether a read's beat is taken this cycle: one is offered, the
/// release has room for its identifier, and no write response is
/// ahead of it.
#[lower]
fn landing(rv: bool, rel_room: Bit, dv: bool) -> Bit {
    rv & rel_room & !dv
}

// begin{run}
#[lower]
impl<
        const A: usize,
        const I: usize,
        const LOGW: usize,
        const H: usize,
        const BASE: usize,
        const DL: usize,
        const CTRL: usize,
    > Unit for Raster<A, I, LOGW, H, BASE, DL, CTRL>
{
    /// Two processes. The first takes the link's answers every cycle:
    /// a write's response and a read's beat come back on channels of
    /// their own and one identifier goes back a cycle, so a write's
    /// is taken first and a read's waits; it also says whether the
    /// pixel under the walk is in the primitive, and whether the
    /// rasteriser is idle. The second is the front end as a
    /// sequence: poll the count until the list is ready, then for
    /// each instruction fetch its six words and walk its box a pixel
    /// a turn, and when the list is drawn write its count back to zero
    /// and poll again.
    async fn run(
        &mut self,
        (grant, done, rdata, ring): (
            Rx<Grant<I>>,
            Rx<Done<I>>,
            Rx<R<32, I>>,
            In<Bit>,
        ),
        (issue, wbeat, release, idle): (
            Tx<Issue<A>>,
            Tx<W<32, 4>>,
            Tx<Grant<I>>,
            Out<Bit>,
        ),
    ) {
        join2(
            async {
                loop {
                    DefaultClock::rising().await;
                    let rel_room = release.ready();
                    let dh = done.head();
                    let dv = done.peek().is_some();
                    let rh = rdata.head();
                    let dgo = dv & rel_room;
                    let got = landing(rdata.peek().is_some(), rel_room, dv);
                    let _ = done.recv_if(rel_room);
                    let _ = rdata.recv_if(rel_room & !dv);
                    let _ = grant.recv_if(grant.peek().is_some());
                    // Whether this pixel is in the primitive: every
                    // pixel of a box, and of a triangle, flat or
                    // shaded, the pixels at which no edge function is
                    // negative.
                    let n0 = !self.e0.get().bit(31);
                    let n1 = !self.e1.get().bit(31);
                    let n2 = !self.e2.get().bit(31);
                    let boxed = Bit::from(self.kind.get() != Kind::Tri)
                        & Bit::from(self.kind.get() != Kind::Shaded);
                    self.hit.set(boxed | (n0 & n1 & n2));
                    // The word this pixel takes: its alpha above a
                    // shaded triangle's three planes here, or above
                    // every other entry's own colour.
                    let shade = channel(self.cr.get())
                        .concat::<8, 16>(channel(self.cg.get()))
                        .concat::<8, 24>(channel(self.cb.get()));
                    let shaded = self.kind.get() == Kind::Shaded;
                    let rgb = mux(shaded, shade, self.colour.get());
                    // The alpha plane's, where the entry has one (issue
                    // 1520), which only the pixel's state carries.
                    let planed = self.son.get() & self.pon.get();
                    let alpha =
                        mux(planed, channel(self.pac.get()), self.alpha.get());
                    self.rgb.set(alpha.concat::<24, 32>(rgb));
                    let open = self.issued.get() - self.answered.get();
                    self.inflight.set(open);
                    with!(self <= {
                        dgo ? answered: self.answered.get() + 1,
                    });
                    // Both a write's response and a read's beat give
                    // an identifier back, and one goes out a cycle.
                    // A read burst gives its identifier back with its
                    // last beat, not with each.
                    if (dgo | (got & rh.last)).to_bool() {
                        release.send(Grant {
                            id: mux(dgo, dh.id, rh.id),
                        });
                    }
                    // Nothing left to draw and nothing left in flight.
                    idle.set(self.finished.get() & Bit::from(open == 0));
                }
            },
            async {
                loop {
                    // The count, which says the list is ready; a zero
                    // means poll again. A read is issued while `ring`
                    // is high, once the link has room for it.
                    until(DefaultClock::rising, || ring.get().to_bool()).await;
                    until(DefaultClock::rising, || issue.ready().to_bool())
                        .await;
                    issue.send(Issue {
                        read: Bit::One,
                        addr: U::<A>::from(CTRL as u32),
                        len: U::<8>::from(0u8),
                        size: U::<3>::from(2u8),
                        burst: BurstKind::Incr,
                        lock: Bit::Zero,
                        cache: U::<4>::from(0u8),
                        prot: U::<3>::from(0u8),
                        qos: U::<4>::from(0u8),
                        region: U::<4>::from(0u8),
                    });
                    until(DefaultClock::rising, || {
                        landing(
                            rdata.peek().is_some(),
                            release.ready(),
                            done.peek().is_some(),
                        )
                        .to_bool()
                    })
                    .await;
                    // A count that is not zero is a new list, so the
                    // last one is no longer what `idle` reports.
                    // Bit 31 says the list is a tile table, and the count
                    // is then its tiles (issue 1255). A flat list is
                    // drawn as one tile of all its entries, straight into
                    // memory.
                    let cw = rdata.head().data;
                    let count = cw.slice::<0, 16>();
                    let tiled = cw.bit(31);
                    with!(self <= {
                        left: count,
                        tiled: tiled,
                        tiles: mux(tiled, count, U::<16>::from(1u8)),
                        tile: U::<16>::from(0u8),
                        n: count,
                        insn: U::<16>::from(0u8),
                        finished:
                            mux(count == 0, self.finished.get(), Bit::Zero),
                        cvalid: U::<64>::from(0u8),
                    });
                    DefaultClock::rising().await;
                    if self.left.get() != 0 {
                      for _ in 0..self.tiles.get().raw() as usize {
                        DefaultClock::rising().await;
                        // A tile's record, as one read burst of its two
                        // words: its first entry and how many it has, then
                        // its top left pixel.
                        if self.tiled.get().to_bool() {
                            until(DefaultClock::rising, || {
                                issue.ready().to_bool()
                            })
                            .await;
                            issue.send(Issue {
                                read: Bit::One,
                                addr: U::<A>::from(DL as u32)
                                    + (self.tile.get().resize::<A>() << 3),
                                len: U::<8>::from(1u8),
                                size: U::<3>::from(2u8),
                                burst: BurstKind::Incr,
                                lock: Bit::Zero,
                                cache: U::<4>::from(0u8),
                                prot: U::<3>::from(0u8),
                                qos: U::<4>::from(0u8),
                                region: U::<4>::from(0u8),
                            });
                            until(DefaultClock::rising, || {
                                landing(
                                    rdata.peek().is_some(),
                                    release.ready(),
                                    done.peek().is_some(),
                                )
                                .to_bool()
                            })
                            .await;
                            let w0 = rdata.head().data;
                            // A scrub, when one is due, goes first, as
                            // one more entry.
                            let due = !self.clean.get();
                            with!(self <= {
                                insn: w0.slice::<0, 16>(),
                                n: w0.slice::<16, 16>()
                                    + mux(
                                        due,
                                        U::<16>::from(1u8),
                                        U::<16>::from(0u8),
                                    ),
                                scrub: due,
                            });
                            until(DefaultClock::rising, || {
                                landing(
                                    rdata.peek().is_some(),
                                    release.ready(),
                                    done.peek().is_some(),
                                )
                                .to_bool()
                            })
                            .await;
                            let w1 = rdata.head().data;
                            let oy = w1.slice::<16, 10>().resize::<16>();
                            // The tile's rows on the screen: 64, or what
                            // is left of the screen below its top.
                            let below = U::<16>::from(H as u32) - oy;
                            let rows = mux(
                                below < U::<16>::from(TILE),
                                below,
                                U::<16>::from(TILE),
                            );
                            // A tile to be loaded from the framebuffer
                            // first takes one more entry for it, after
                            // the scrub (issue 993).
                            let load = w1.bit(26);
                            with!(self <= {
                                ox: w1.slice::<0, 10>().resize::<16>(),
                                oy: oy,
                                th: rows.resize::<8>(),
                                tile: self.tile.get() + 1,
                                load: load,
                                n: self.n.get()
                                    + mux(
                                        load,
                                        U::<16>::from(1u8),
                                        U::<16>::from(0u8),
                                    ),
                            });
                            DefaultClock::rising().await;
                        }
                        for _ in 0..self.n.get().raw() as usize {
                            // An edge for the instruction's index to
                            // read back, and for the sequence to
                            // begin a turn with.
                            DefaultClock::rising().await;
                            // A scrub fetches nothing: its box is the
                            // tile, and it tests no depth.
                            if self.scrub.get().to_bool() {
                                let zero16 = U::<16>::from(0u8);
                                let last = U::<16>::from(TILE - 1);
                                with!(self <= {
                                    x: zero16,
                                    y: zero16,
                                    xa: zero16,
                                    xb: last,
                                    yb: last,
                                    pa: U::<12>::from(0u8),
                                    zon: Bit::Zero,
                                    son: Bit::Zero,
                                    tex_on: Bit::Zero,
                                });
                            }
                            // A load fetches nothing either: its box is
                            // the tile where it is on the screen, whose
                            // rows it reads from the framebuffer.
                            if (!self.scrub.get() & self.load.get()).to_bool()
                            {
                                let (tx, ty) = (self.ox.get(), self.oy.get());
                                let th = self.th.get().resize::<16>();
                                with!(self <= {
                                    x: tx,
                                    y: ty,
                                    xa: tx,
                                    xb: tx + U::<16>::from(TILE - 1),
                                    yb: ty + th - U::<16>::from(1u8),
                                    pa: U::<12>::from(0u8),
                                    zon: Bit::Zero,
                                    son: Bit::Zero,
                                    tex_on: Bit::Zero,
                                });
                            }
                            if !(self.scrub.get() | self.load.get()).to_bool() {
                              self.word.set(U::<5>::from(0u8));
                              // The instruction's sixteen words, as one
                              // read burst of sixteen beats, each latched
                              // as it lands. An entry is sixteen words from
                              // an address a multiple of sixty-four, so the
                              // burst never crosses anything a burst may
                              // not.
                              until(DefaultClock::rising, || {
                                  issue.ready().to_bool()
                              })
                              .await;
                              issue.send(Issue {
                                  read: Bit::One,
                                  addr: U::<A>::from(DL as u32)
                                      + mux(
                                          self.tiled.get(),
                                          U::<A>::from(ENTRIES_AT),
                                          U::<A>::from(0u32),
                                      )
                                      + (self.insn.get().resize::<A>()
                                          << SHIFT),
                                  len: U::<8>::from(15u8),
                                  size: U::<3>::from(2u8),
                                  burst: BurstKind::Incr,
                                  lock: Bit::Zero,
                                  cache: U::<4>::from(0u8),
                                  prot: U::<3>::from(0u8),
                                  qos: U::<4>::from(0u8),
                                  region: U::<4>::from(0u8),
                              });
                              for _ in 0..16 {
                                  until(DefaultClock::rising, || {
                                      landing(
                                          rdata.peek().is_some(),
                                          release.ready(),
                                          done.peek().is_some(),
                                      )
                                      .to_bool()
                                  })
                                  .await;
                                  let rh = rdata.head();
                                  self.word.set(self.word.get() + 1);
                                  let word0 = rh.data.slice::<0, 2>();
                                  // The box. A clear says only its
                                  // colour, so its box is the screen,
                                  // which the rasteriser knows from its
                                  // own type; a rectangle and a triangle
                                  // carry theirs.
                                  let clearing =
                                      self.skind.get() == Kind::Clear;
                                  let zero16 = U::<16>::from(0u8);
                                  let last_x = U::<16>::from(
                                      ((1usize << LOGW) - 1) as u32,
                                  );
                                  let last_y = U::<16>::from((H - 1) as u32);
                                  let wx = mux(
                                      clearing,
                                      zero16,
                                      self.sx0.get().resize::<16>(),
                                  );
                                  let wy = mux(
                                      clearing,
                                      zero16,
                                      self.sy0.get().resize::<16>(),
                                  );
                                  let bx1 = mux(
                                      clearing,
                                      last_x,
                                      self.sx1.get().resize::<16>(),
                                  );
                                  let by1 = mux(
                                      clearing,
                                      last_y,
                                      self.sy1.get().resize::<16>(),
                                  );
                                  // The kind, from the word's low two
                                  // bits: a clear, a rectangle, or a
                                  // triangle, flat or shaded.
                                  let tri_kind =
                                      mux(word0 == 2, Kind::Tri, Kind::Shaded);
                                  let rect_or_tri =
                                      mux(word0 == 1, Kind::Rect, tri_kind);
                                  if self.word.get() == 0 {
                                      with!(self <= {
                                          skind: mux(
                                              word0 == 0,
                                              Kind::Clear,
                                              rect_or_tri,
                                          ),
                                          scol: rh.data.slice::<2, 24>(),
                                      });
                                  }
                                  if self.word.get() == 1 {
                                      with!(self <= {
                                          sx0: rh.data.slice::<0, 10>(),
                                          sy0: rh.data.slice::<16, 10>(),
                                      });
                                  }
                                  if self.word.get() == 2 {
                                      with!(self <= {
                                          sx1: rh.data.slice::<0, 10>(),
                                          sy1: rh.data.slice::<16, 10>(),
                                      });
                                  }
                                  if self.word.get() == 3 {
                                      with!(self <= {
                                          sax: rh.data.slice::<0, 16>(),
                                          say: rh.data.slice::<16, 16>(),
                                      });
                                  }
                                  if self.word.get() == 4 {
                                      with!(self <= {
                                          sbx: rh.data.slice::<0, 16>(),
                                          sby: rh.data.slice::<16, 16>(),
                                      });
                                  }
                                  if self.word.get() == 5 {
                                      with!(self <= {
                                          kind: self.skind.get(),
                                          colour: self.scol.get(),
                                          x: wx,
                                          y: wy,
                                          xa: wx,
                                          pa: wy
                                              .slice::<0, 6>()
                                              .concat::<6, 12>(
                                                  wx.slice::<0, 6>(),
                                              ),
                                          xb: bx1,
                                          yb: by1,
                                          scx: rh.data.slice::<0, 16>(),
                                          scy: rh.data.slice::<16, 16>(),
                                      });
                                  }
                                  // A shaded triangle's planes, each its
                                  // value at the box's first pixel and its
                                  // two steps, which the host worked out,
                                  // so they go straight to the walk. The
                                  // other entries carry zeros here.
                                  let v = rh.data;
                                  if self.word.get() == 6 {
                                      with!(self <= { cr: v, lr: v });
                                  }
                                  if self.word.get() == 7 {
                                      self.crx.set(v);
                                  }
                                  if self.word.get() == 8 {
                                      self.cry.set(v);
                                  }
                                  if self.word.get() == 9 {
                                      with!(self <= { cg: v, lg: v });
                                  }
                                  if self.word.get() == 10 {
                                      self.cgx.set(v);
                                  }
                                  if self.word.get() == 11 {
                                      self.cgy.set(v);
                                  }
                                  if self.word.get() == 12 {
                                      with!(self <= { cb: v, lb: v });
                                  }
                                  if self.word.get() == 13 {
                                      self.cbx.set(v);
                                  }
                                  if self.word.get() == 14 {
                                      self.cby.set(v);
                                  }
                                  // Every entry's alpha, its depth bits,
                                  // and whether a second slot follows,
                                  // for depth or for the pixel's state
                                  // (issue 993).
                                  if self.word.get() == 15 {
                                      with!(self <= {
                                          alpha: v.slice::<0, 8>(),
                                          deep: v.bit(8)
                                              | v.bit(13)
                                              | v.bit(14),
                                          texd: v.bit(14),
                                          tex_on: v.bit(14) & self.tiled.get(),
                                          zon: v.bit(8) & self.tiled.get(),

                                          son: v.bit(13) & self.tiled.get(),
                                          zfunc: v.slice::<9, 3>(),
                                          zwrite: v.bit(12),
                                      });
                                  }
                              }
                              // An entry that tests depth is followed by
                              // its depth plane's slot (issue 992), read in
                              // a tile and passed over in a flat list, which
                              // has no depth.
                              // An edge first, for word 15's bits to be read.
                              DefaultClock::rising().await;
                              if self.deep.get().to_bool() {
                                  if self.tiled.get().to_bool() {
                                      until(DefaultClock::rising, || {
                                          issue.ready().to_bool()
                                      })
                                      .await;
                                      issue.send(Issue {
                                          read: Bit::One,
                                          addr: U::<A>::from(DL as u32)
                                              + U::<A>::from(ENTRIES_AT)
                                              + ((self.insn.get() + 1)
                                                  .resize::<A>()
                                                  << SHIFT),
                                          len: U::<8>::from(15u8),
                                          size: U::<3>::from(2u8),
                                          burst: BurstKind::Incr,
                                          lock: Bit::Zero,
                                          cache: U::<4>::from(0u8),
                                          prot: U::<3>::from(0u8),
                                          qos: U::<4>::from(0u8),
                                          region: U::<4>::from(0u8),
                                      });
                                      until(DefaultClock::rising, || {
                                          landing(
                                              rdata.peek().is_some(),
                                              release.ready(),
                                              done.peek().is_some(),
                                          )
                                          .to_bool()
                                      })
                                      .await;
                                      let z0 = rdata.head().data;
                                      with!(self <= { zc: z0, zr: z0 });
                                      until(DefaultClock::rising, || {
                                          landing(
                                              rdata.peek().is_some(),
                                              release.ready(),
                                              done.peek().is_some(),
                                          )
                                          .to_bool()
                                      })
                                      .await;
                                      self.zdx.set(rdata.head().data);
                                      until(DefaultClock::rising, || {
                                          landing(
                                              rdata.peek().is_some(),
                                              release.ready(),
                                              done.peek().is_some(),
                                          )
                                          .to_bool()
                                      })
                                      .await;
                                      self.zdy.set(rdata.head().data);
                                      // The pixel's state, in the slot's
                                      // words 3 and 4 (issue 993).
                                      until(DefaultClock::rising, || {
                                          landing(
                                              rdata.peek().is_some(),
                                              release.ready(),
                                              done.peek().is_some(),
                                          )
                                          .to_bool()
                                      })
                                      .await;
                                      let w3 = rdata.head().data;
                                      with!(self <= {
                                          bon: w3.bit(0),
                                          sfac: w3.slice::<4, 4>(),
                                          dfac: w3.slice::<8, 4>(),
                                      });
                                      until(DefaultClock::rising, || {
                                          landing(
                                              rdata.peek().is_some(),
                                              release.ready(),
                                              done.peek().is_some(),
                                          )
                                          .to_bool()
                                      })
                                      .await;
                                      let w4 = rdata.head().data;
                                      with!(self <= {
                                          aon: w4.bit(0),
                                          afunc: w4.slice::<1, 3>(),
                                          aref: w4.slice::<8, 8>(),
                                          cmask: w4.slice::<16, 4>(),
                                      });
                                      // The logic operation, word 5 (issue
                                      // 998).
                                      until(DefaultClock::rising, || {
                                          landing(
                                              rdata.peek().is_some(),
                                              release.ready(),
                                              done.peek().is_some(),
                                          )
                                          .to_bool()
                                      })
                                      .await;
                                      let w5 = rdata.head().data;
                                      with!(self <= {
                                          lon: w5.bit(0),
                                          lop: w5.slice::<1, 4>(),
                                      });
                                      // Fog, words 6 to 9 (issue 998): its
                                      // factor's plane, then whether it is
                                      // on and its colour.
                                      until(DefaultClock::rising, || {
                                          landing(
                                              rdata.peek().is_some(),
                                              release.ready(),
                                              done.peek().is_some(),
                                          )
                                          .to_bool()
                                      })
                                      .await;
                                      let f0 = rdata.head().data;
                                      with!(self <= { fc: f0, fr: f0 });
                                      until(DefaultClock::rising, || {
                                          landing(
                                              rdata.peek().is_some(),
                                              release.ready(),
                                              done.peek().is_some(),
                                          )
                                          .to_bool()
                                      })
                                      .await;
                                      self.fdx.set(rdata.head().data);
                                      until(DefaultClock::rising, || {
                                          landing(
                                              rdata.peek().is_some(),
                                              release.ready(),
                                              done.peek().is_some(),
                                          )
                                          .to_bool()
                                      })
                                      .await;
                                      self.fdy.set(rdata.head().data);
                                      until(DefaultClock::rising, || {
                                          landing(
                                              rdata.peek().is_some(),
                                              release.ready(),
                                              done.peek().is_some(),
                                          )
                                          .to_bool()
                                      })
                                      .await;
                                      let w9 = rdata.head().data;
                                      with!(self <= {
                                          fon: w9.bit(0),
                                          fcol: w9.slice::<8, 24>(),
                                      });
                                      // The stencil, words 10 and 11 (issue
                                      // 998).
                                      until(DefaultClock::rising, || {
                                          landing(
                                              rdata.peek().is_some(),
                                              release.ready(),
                                              done.peek().is_some(),
                                          )
                                          .to_bool()
                                      })
                                      .await;
                                      let w10 = rdata.head().data;
                                      with!(self <= {
                                          sten: w10.bit(0),
                                          sfunc: w10.slice::<1, 3>(),
                                          sref: w10.slice::<8, 8>(),
                                          smask: w10.slice::<16, 8>(),
                                          swmask: w10.slice::<24, 8>(),
                                      });
                                      until(DefaultClock::rising, || {
                                          landing(
                                              rdata.peek().is_some(),
                                              release.ready(),
                                              done.peek().is_some(),
                                          )
                                          .to_bool()
                                      })
                                      .await;
                                      let w11 = rdata.head().data;
                                      with!(self <= {
                                          sfail: w11.slice::<0, 3>(),
                                          szfail: w11.slice::<3, 3>(),
                                          szpass: w11.slice::<6, 3>(),
                                      });
                                      // A shaded triangle's alpha plane,
                                      // words 12 to 15 (issue 1520): the
                                      // plane, then whether it is on.
                                      until(DefaultClock::rising, || {
                                          landing(
                                              rdata.peek().is_some(),
                                              release.ready(),
                                              done.peek().is_some(),
                                          )
                                          .to_bool()
                                      })
                                      .await;
                                      let a0 = rdata.head().data;
                                      with!(self <= { pac: a0, par: a0 });
                                      until(DefaultClock::rising, || {
                                          landing(
                                              rdata.peek().is_some(),
                                              release.ready(),
                                              done.peek().is_some(),
                                          )
                                          .to_bool()
                                      })
                                      .await;
                                      self.padx.set(rdata.head().data);
                                      until(DefaultClock::rising, || {
                                          landing(
                                              rdata.peek().is_some(),
                                              release.ready(),
                                              done.peek().is_some(),
                                          )
                                          .to_bool()
                                      })
                                      .await;
                                      self.pady.set(rdata.head().data);
                                      until(DefaultClock::rising, || {
                                          landing(
                                              rdata.peek().is_some(),
                                              release.ready(),
                                              done.peek().is_some(),
                                          )
                                          .to_bool()
                                      })
                                      .await;
                                      self.pon.set(rdata.head().data.bit(0));
                                  }
                                  // A textured entry's two slots more
                                  // (issue 997). In a tile, slot A's
                                  // sixteen words, slot B's first six and
                                  // the first two of the descriptor slot A
                                  // names; a flat list passes them over and
                                  // draws the entry untextured. A plane's
                                  // low word lands first and is held for
                                  // its high one.
                                  if self.tex_on.get().to_bool() {
                                      until(DefaultClock::rising, || {
                                          issue.ready().to_bool()
                                      })
                                      .await;
                                      issue.send(Issue {
                                          read: Bit::One,
                                          addr: U::<A>::from(DL as u32)
                                              + U::<A>::from(ENTRIES_AT)
                                              + ((self.insn.get() + 2)
                                                  .resize::<A>()
                                                  << SHIFT),
                                          len: U::<8>::from(15u8),
                                          size: U::<3>::from(2u8),
                                          burst: BurstKind::Incr,
                                          lock: Bit::Zero,
                                          cache: U::<4>::from(0u8),
                                          prot: U::<3>::from(0u8),
                                          qos: U::<4>::from(0u8),
                                          region: U::<4>::from(0u8),
                                      });
                                      self.word.set(U::<5>::from(0u8));
                                      for _ in 0..16 {
                                          until(DefaultClock::rising, || {
                                              landing(
                                                  rdata.peek().is_some(),
                                                  release.ready(),
                                                  done.peek().is_some(),
                                              )
                                              .to_bool()
                                          })
                                          .await;
                                          let sa = rdata.head().data;
                                          let sat = self.word.get();
                                          let sal = self.tlo.get();
                                          let sa64 = sa.concat::<32, 64>(sal);
                                          with!(self <= {
                                              tlo: sa,
                                              word: sat + 1,
                                          });
                                          if sat == 1 {
                                              self.tuc.set(sa64);
                                              self.tur.set(sa64);
                                          }
                                          if sat == 3 {
                                              self.tudx.set(sa64);
                                          }
                                          if sat == 5 {
                                              self.tudy.set(sa64);
                                          }
                                          if sat == 7 {
                                              self.tvc.set(sa64);
                                              self.tvr.set(sa64);
                                          }
                                          if sat == 9 {
                                              self.tvdx.set(sa64);
                                          }
                                          if sat == 11 {
                                              self.tvdy.set(sa64);
                                          }
                                          if sat == 12 {
                                              let sak = sa.slice::<0, 8>();
                                              self.tlodk.set(sak);
                                          }
                                          if sat == 13 {
                                              self.tdesc.set(sa);
                                          }
                                          if sat == 14 {
                                              let sae = sa.slice::<0, 3>();
                                              self.tenv.set(sae);
                                          }
                                          if sat == 15 {
                                              self.tenvc.set(sa);
                                          }
                                      }
                                      until(DefaultClock::rising, || {
                                          issue.ready().to_bool()
                                      })
                                      .await;
                                      issue.send(Issue {
                                          read: Bit::One,
                                          addr: U::<A>::from(DL as u32)
                                              + U::<A>::from(ENTRIES_AT)
                                              + ((self.insn.get() + 3)
                                                  .resize::<A>()
                                                  << SHIFT),
                                          len: U::<8>::from(13u8),
                                          size: U::<3>::from(2u8),
                                          burst: BurstKind::Incr,
                                          lock: Bit::Zero,
                                          cache: U::<4>::from(0u8),
                                          prot: U::<3>::from(0u8),
                                          qos: U::<4>::from(0u8),
                                          region: U::<4>::from(0u8),
                                      });
                                      self.word.set(U::<5>::from(0u8));
                                      for _ in 0..14 {
                                          until(DefaultClock::rising, || {
                                              landing(
                                                  rdata.peek().is_some(),
                                                  release.ready(),
                                                  done.peek().is_some(),
                                              )
                                              .to_bool()
                                          })
                                          .await;
                                          let sb = rdata.head().data;
                                          let sbt = self.word.get();
                                          let sbl = self.tlo.get();
                                          let sb64 = sb.concat::<32, 64>(sbl);
                                          with!(self <= {
                                              tlo: sb,
                                              word: sbt + 1,
                                          });
                                          if sbt == 1 {
                                              self.tqc.set(sb64);
                                              self.tqr.set(sb64);
                                          }
                                          if sbt == 3 {
                                              self.tqdx.set(sb64);
                                          }
                                          if sbt == 5 {
                                              self.tqdy.set(sb64);
                                          }
                                          // The numerators: the first two step
                                          // down, the others across.
                                          if sbt == 6 {
                                              self.tnux.set(sb);
                                          }
                                          if sbt == 7 {
                                              self.tnuxd.set(sb);
                                          }
                                          if sbt == 8 {
                                              self.tnvx.set(sb);
                                          }
                                          if sbt == 9 {
                                              self.tnvxd.set(sb);
                                          }
                                          if sbt == 10 {
                                              self.tnuy0.set(sb);
                                              self.tnuy.set(sb);
                                          }
                                          if sbt == 11 {
                                              self.tnuyd.set(sb);
                                          }
                                          if sbt == 12 {
                                              self.tnvy0.set(sb);
                                              self.tnvy.set(sb);
                                          }
                                          if sbt == 13 {
                                              self.tnvyd.set(sb);
                                          }
                                      }
                                      // The descriptor's twelve words: the
                                      // sides, levels, filters, wrap modes and
                                      // class, then each level's address.
                                      until(DefaultClock::rising, || {
                                          issue.ready().to_bool()
                                      })
                                      .await;
                                      let tdv = self.tdesc.get();
                                      let tdq = tdv.slice::<0, A>();
                                      issue.send(Issue {
                                          read: Bit::One,
                                          addr: tdq,
                                          len: U::<8>::from(11u8),
                                          size: U::<3>::from(2u8),
                                          burst: BurstKind::Incr,
                                          lock: Bit::Zero,
                                          cache: U::<4>::from(0u8),
                                          prot: U::<3>::from(0u8),
                                          qos: U::<4>::from(0u8),
                                          region: U::<4>::from(0u8),
                                      });
                                      self.word.set(U::<5>::from(0u8));
                                      for _ in 0..12 {
                                          until(DefaultClock::rising, || {
                                              landing(
                                                  rdata.peek().is_some(),
                                                  release.ready(),
                                                  done.peek().is_some(),
                                              )
                                              .to_bool()
                                          })
                                          .await;
                                          let dw = rdata.head().data;
                                          let dk = self.word.get();
                                          self.word.set(dk + 1);
                                          if dk == 0 {
                                              with!(self <= {
                                                  tlogw: dw.slice::<0, 4>(),
                                                  tlogh: dw.slice::<4, 4>(),
                                                  tlevels: dw.slice::<8, 4>(),
                                                  tcs: dw.bit(12),
                                                  tct: dw.bit(13),
                                                  tmin: dw.slice::<16, 3>(),
                                                  tmag: dw.bit(19),
                                                  tclass: dw.slice::<20, 3>(),
                                              });
                                          }
                                          let dl = (dk - 1).slice::<0, 4>();
                                          let dput = Bit::from(dk != 0);
                                          with!(self <= {
                                              dput ? { tlvl.at(dl): dw },
                                          });
                                      }
                                  }
                                  self.insn.set(
                                      self.insn.get()
                                          + mux(
                                              self.texd.get(),
                                              U::<16>::from(3u8),
                                              U::<16>::from(1u8),
                                          ),
                                  );
                              }
                            }
                            // The setup the walk asks for: per edge, the
                            // two steps and the value at the box's first
                            // pixel. It takes three cycles, so that no
                            // cycle holds more than one multiplication
                            // (issue 1034). A vertex is sixteenths of a
                            // pixel in two's complement and is widened by
                            // its sign; the box's first pixel is a screen
                            // coordinate, sampled at its centre, sixteen
                            // times it and eight more (issue 988).
                            DefaultClock::rising().await;
                            let ax = self.sax.get().sext::<32>();
                            let ay = self.say.get().sext::<32>();
                            let bx = self.sbx.get().sext::<32>();
                            let by = self.sby.get().sext::<32>();
                            let cx = self.scx.get().sext::<32>();
                            let cy = self.scy.get().sext::<32>();
                            let half = U::<32>::from(8u8);
                            let x16 = self.x.get().resize::<32>() << 4;
                            let y16 = self.y.get().resize::<32>() << 4;
                            let (sx, sy) = (x16 + half, y16 + half);
                            let zero = U::<32>::from(0u8);
                            // The steps, and each edge's distances to
                            // the box's first pixel.
                            with!(self <= {
                                d0x: zero - (by - ay), d0y: bx - ax,
                                d1x: zero - (cy - by), d1y: cx - bx,
                                d2x: zero - (ay - cy), d2y: ax - cx,
                                u0x: sx - ax, u0y: sy - ay,
                                u1x: sx - bx, u1y: sy - by,
                                u2x: sx - cx, u2y: sy - cy,
                            });
                            // The products, each in the register it read;
                            // the steps become a pixel's, sixteen of the
                            // vertices' units; and each edge's place in
                            // the fill rule.
                            DefaultClock::rising().await;
                            let (d0x, d0y) = (self.d0x.get(), self.d0y.get());
                            let (d1x, d1y) = (self.d1x.get(), self.d1y.get());
                            let (d2x, d2y) = (self.d2x.get(), self.d2y.get());
                            with!(self <= {
                                u0x: d0x.mul::<32>(self.u0x.get()),
                                u0y: d0y.mul::<32>(self.u0y.get()),
                                u1x: d1x.mul::<32>(self.u1x.get()),
                                u1y: d1y.mul::<32>(self.u1y.get()),
                                u2x: d2x.mul::<32>(self.u2x.get()),
                                u2y: d2y.mul::<32>(self.u2y.get()),
                                d0x: d0x << 4, d0y: d0y << 4,
                                d1x: d1x << 4, d1y: d1y << 4,
                                d2x: d2x << 4, d2y: d2y << 4,
                                tl0: top_left(d0x, d0y),
                                tl1: top_left(d1x, d1y),
                                tl2: top_left(d2x, d2y),
                            });
                            // Each edge at the box's first pixel: the
                            // sum of its two products, less one on an
                            // edge that is neither top nor left, so that
                            // a centre exactly on it fails the sign test.
                            DefaultClock::rising().await;
                            let (keep, less) =
                                (U::<32>::from(0u8), U::<32>::from(1u8));
                            let s0 = self.u0x.get() + self.u0y.get()
                                - mux(self.tl0.get(), keep, less);
                            let s1 = self.u1x.get() + self.u1y.get()
                                - mux(self.tl1.get(), keep, less);
                            let s2 = self.u2x.get() + self.u2y.get()
                                - mux(self.tl2.get(), keep, less);
                            with!(self <= {
                                e0: s0, r0: s0,
                                e1: s1, r1: s1,
                                e2: s2, r2: s2,
                            });
                            // An edge, for the walk to read back.
                            DefaultClock::rising().await;
                            // The walk: every row of the box, and
                            // every column of the row, a pixel a
                            // turn. A pixel is written when there is
                            // room for the burst and for its beat; the
                            // turn ends when the pixel wanted no
                            // write, or when its write went out.
                            for _ in self.y.get().raw() as usize
                                ..=self.yb.get().raw() as usize
                            {
                                DefaultClock::rising().await;
                                // A load reads the row from the
                                // framebuffer, one burst of the tile's
                                // width, and takes a beat a pixel.
                                if self.load.get().to_bool() {
                                    until(DefaultClock::rising, || {
                                        issue.ready().to_bool()
                                    })
                                    .await;
                                    let row = (self.y.get().resize::<A>()
                                        << LOGW)
                                        + self.xa.get().resize::<A>();
                                    issue.send(Issue {
                                        read: Bit::One,
                                        addr: (row << WORD)
                                            + U::<A>::from(BASE as u32),
                                        len: U::<8>::from(TILE_LEN),
                                        size: U::<3>::from(2u8),
                                        burst: BurstKind::Incr,
                                        lock: Bit::Zero,
                                        cache: U::<4>::from(0u8),
                                        prot: U::<3>::from(0u8),
                                        qos: U::<4>::from(0u8),
                                        region: U::<4>::from(0u8),
                                    });
                                }
                                for _ in self.xa.get().raw() as usize
                                    ..=self.xb.get().raw() as usize
                                {
                                    // A burst under way needs room for
                                    // its next beat; a pixel that would
                                    // start one, room for the burst and
                                    // its first beat.
                                    // In a tile the pixel goes to the
                                    // bank, which takes one a cycle; in
                                    // a load, when its beat lands.
                                    until(DefaultClock::rising, || {
                                        ((self.tiled.get() & !self.load.get())
                                            | (self.load.get()
                                                & landing(
                                                    rdata.peek().is_some(),
                                                    release.ready(),
                                                    done.peek().is_some(),
                                                ))
                                            | (!self.tiled.get()
                                                & ((Bit::from(
                                                    self.beats.get() != 0,
                                                ) & wbeat.ready())
                                                    | (Bit::from(
                                                        self.beats.get() == 0,
                                                    ) & (!self.hit.get()
                                                        | (issue.ready()
                                                            & wbeat
                                                                .ready()))))))
                                        .to_bool()
                                    })
                                    .await;
                                    // A textured pixel (issue 997) takes
                                    // its texel first, a turn for each
                                    // step of `tex::texel_uv`, `tex::lod`
                                    // and `tex::sample`: q normalised,
                                    // and the numerators' largest; the
                                    // reciprocal's first guess, and the
                                    // largest normalised; its Newton
                                    // step's error, and the two lines'
                                    // products for `log2`; the
                                    // reciprocal, and the level of
                                    // detail; `u q` and `v q` times it,
                                    // and the levels and filter it picks;
                                    // the shift back and the clamp. Then
                                    // each texel the filters read, one or
                                    // four a level and one level or two,
                                    // its own turns: its level's
                                    // coordinates; its column and row,
                                    // wrapped, and its weight; its
                                    // address; the cache read, and a
                                    // refill on a miss; the weight times
                                    // each channel; and the sum. Then
                                    // the two levels' blend, and the
                                    // environment, its products and
                                    // then their divide.
                                    if (self.tex_on.get() & self.hit.get())
                                        .to_bool()
                                    {
                                        let tq = self.tqc.get();
                                        let one64 = U::<64>::from(1u8);
                                        let tq1 = mux(tq == 0, one64, tq);
                                        let nm = biggest(
                                            self.tnux.get(),
                                            self.tnvx.get(),
                                            self.tnuy.get(),
                                            self.tnvy.get(),
                                        );
                                        with!(self <= {
                                            tn: lz64(tq1),
                                            tx: norm64(tq1).slice::<32, 32>(),
                                            mlm: nm,
                                        });
                                        DefaultClock::rising().await;
                                        let tx0 = self.tx.get();
                                        let tk = tx0.slice::<26, 5>();
                                        let tt10 = tx0.slice::<16, 10>();
                                        let tt = tt10.resize::<22>();
                                        let tf0 = seed_fall(tk).resize::<22>();
                                        let tf = tf0.mul::<22>(tt);
                                        let tf12 = tf.slice::<10, 12>();
                                        let tfs = tf12.resize::<17>();
                                        let mlw = self.mlm.get().resize::<64>();
                                        let mlo = U::<64>::from(1u64 << 32);
                                        let mlz0 = mlw == 0;
                                        let mls = mlw << 32usize;
                                        let mlu = mux(mlz0, mlo, mls);
                                        with!(self <= {
                                            tr0: seed_start(tk) - tfs,
                                            mlz: lz64(mlu),
                                            mlx: norm64(mlu).slice::<32, 32>(),
                                        });
                                        DefaultClock::rising().await;
                                        let tr0v = self.tr0.get();
                                        let tx50 = self.tx.get().resize::<50>();
                                        let r050 = tr0v.resize::<50>();
                                        let tp = tx50.mul::<50>(r050);
                                        let one49 = U::<50>::from(1u64 << 49);
                                        let lqx = self.tx.get();
                                        let lqk = lqx.slice::<26, 5>();
                                        let lqb = log_rise(lqk);
                                        let lqt = lqx.slice::<16, 10>();
                                        let lmx = self.mlx.get();
                                        let lmk = lmx.slice::<26, 5>();
                                        let lmb = log_rise(lmk);
                                        let lmt = lmx.slice::<16, 10>();
                                        let lqb22 = lqb.resize::<22>();
                                        let lmb22 = lmb.resize::<22>();
                                        let lqt22 = lqt.resize::<22>();
                                        let lmt22 = lmt.resize::<22>();
                                        with!(self <= {
                                            te: one49 - tp,
                                            mpq: lqb22.mul::<22>(lqt22),
                                            mpm: lmb22.mul::<22>(lmt22),
                                        });
                                        DefaultClock::rising().await;
                                        let tr0w = self.tr0.get();
                                        let r068 = tr0w.resize::<68>();
                                        let te68 = self.te.get().resize::<68>();
                                        let tre = r068.mul::<68>(te68);
                                        // Each line's fraction: its start
                                        // and its product, rounded.
                                        let fqx = self.tx.get();
                                        let fqk = fqx.slice::<26, 5>();
                                        let fqa = log_start(fqk).resize::<32>();
                                        let fqp = self.mpq.get();
                                        let fqb = fqp.slice::<10, 12>();
                                        let fq0 = fqa + fqb.resize::<32>();
                                        let fq = (fq0 + 128) >> 8usize;
                                        let fmx = self.mlx.get();
                                        let fmk = fmx.slice::<26, 5>();
                                        let fma = log_start(fmk).resize::<32>();
                                        let fmp = self.mpm.get();
                                        let fmb = fmp.slice::<10, 12>();
                                        let fm0 = fma + fmb.resize::<32>();
                                        let fm = (fm0 + 128) >> 8usize;
                                        let lmv = self.mlm.get();
                                        let lnone = Bit::from(lmv == 0);
                                        let lod = lod_of(
                                            self.mlz.get(),
                                            fm,
                                            self.tlodk.get(),
                                            self.tn.get(),
                                            fq,
                                            lnone,
                                        );
                                        with!(self <= {
                                            trc: tre.slice::<40, 28>(),
                                            mlod: lod,
                                        });
                                        DefaultClock::rising().await;
                                        // A signed plane times the reciprocal:
                                        // each 32-bit half of its bits times
                                        // it, as unsigned; and a turn later the
                                        // two added, less the reciprocal where
                                        // the sign bit stood for 2^64 rather
                                        // than -2^64. One product of 64 bits in
                                        // a turn missed the clock.
                                        let r64 = self.trc.get().resize::<64>();
                                        let tuv = self.tuc.get();
                                        let tvv = self.tvc.get();
                                        let ul32 = tuv.slice::<0, 32>();
                                        let uh32 = tuv.slice::<32, 32>();
                                        let vl32 = tvv.slice::<0, 32>();
                                        let vh32 = tvv.slice::<32, 32>();
                                        let ul = ul32.resize::<64>();
                                        let uh = uh32.resize::<64>();
                                        let vl = vl32.resize::<64>();
                                        let vh = vh32.resize::<64>();
                                        let pick = pick_level(
                                            self.mlod.get(),
                                            self.tmin.get(),
                                            self.tmag.get(),
                                            self.tlevels.get(),
                                        );
                                        with!(self <= {
                                            tpul: ul.mul::<64>(r64),
                                            tpuh: uh.mul::<64>(r64),
                                            tpvl: vl.mul::<64>(r64),
                                            tpvh: vh.mul::<64>(r64),
                                            mlev1: pick.slice::<0, 4>(),
                                            mlev2: pick.slice::<4, 4>(),
                                            mtwo: pick.bit(8),
                                            mlin: pick.bit(9),
                                            mfr: pick.slice::<10, 8>(),
                                            mnt: pick.slice::<18, 4>(),
                                        });
                                        DefaultClock::rising().await;
                                        let rcs = self.trc.get().resize::<96>();
                                        let rc = rcs << 64usize;
                                        let n96 = U::<96>::from(0u8);
                                        let su = self.tuc.get().bit(63);
                                        let sv = self.tvc.get().bit(63);
                                        let cu = mux(su, rc, n96);
                                        let cv = mux(sv, rc, n96);
                                        let pul = self.tpul.get();
                                        let puh = self.tpuh.get();
                                        let pvl = self.tpvl.get();
                                        let pvh = self.tpvh.get();
                                        let pu96 = pul.resize::<96>() - cu;
                                        let pv96 = pvl.resize::<96>() - cv;
                                        let puh96 = puh.resize::<96>();
                                        let pvh96 = pvh.resize::<96>();
                                        let ph96 = puh96 << 32usize;
                                        let pw96 = pvh96 << 32usize;
                                        self.tpu.set(pu96 + ph96);
                                        self.tpv.set(pv96 + pw96);
                                        DefaultClock::rising().await;
                                        let tnz = self.tn.get();
                                        let tsh7 = U::<7>::from(64u8) - tnz;
                                        let tsh = tsh7.raw() as usize;
                                        let pu = sra(self.tpu.get(), tsh);
                                        let pv = sra(self.tpv.get(), tsh);
                                        with!(self <= {
                                            tiu: clamp30(pu),
                                            tiv: clamp30(pv),
                                            mk: U::<4>::from(0u8),
                                        });
                                        DefaultClock::rising().await;
                                    }
                                    // Each texel the filters read, none
                                    // for a pixel that is not textured.
                                    for _ in 0..self.mnt.get().raw() as usize {
                                        // Which texel this is: its pass,
                                        // the level, and its place among
                                        // the four; and its level's
                                        // coordinates, sides and address.
                                        let ik = self.mk.get();
                                        let il = self.mlin.get();
                                        let ip = mux(il, ik.bit(2), ik.bit(0));
                                        let z2 = U::<2>::from(0u8);
                                        let ik2 = ik.slice::<0, 2>();
                                        let it = mux(il, ik2, z2);
                                        let iv2 = self.mlev2.get();
                                        let iv1 = self.mlev1.get();
                                        let lev = mux(ip, iv2, iv1);
                                        let lsh = lev.raw() as usize;
                                        let lw = self.tlogw.get();
                                        let lh = self.tlogh.get();
                                        let z4 = U::<4>::from(0u8);
                                        with!(self <= {
                                            mpass: ip,
                                            mt: it,
                                            mus: sra(self.tiu.get(), lsh),
                                            mvs: sra(self.tiv.get(), lsh),
                                            mlw: mux(lw < lev, z4, lw - lev),
                                            mlh: mux(lh < lev, z4, lh - lev),
                                            mbase: self.tlvl.read(lev),
                                        });
                                        DefaultClock::rising().await;
                                        // Linear takes the texels about the
                                        // point half a texel back, weighted
                                        // by the fraction past it.
                                        let jl = self.mlin.get();
                                        let z32 = U::<32>::from(0u8);
                                        let h32 = U::<32>::from(128u8);
                                        let back = mux(jl, h32, z32);
                                        let ju = self.mus.get() - back;
                                        let jv = self.mvs.get() - back;
                                        let jt = self.mt.get();
                                        let z8 = U::<8>::from(0u8);
                                        let ua = ju.slice::<0, 8>();
                                        let va = jv.slice::<0, 8>();
                                        let fa = mux(jl, ua, z8).resize::<9>();
                                        let fb = mux(jl, va, z8).resize::<9>();
                                        let j256 = U::<9>::from(256u32);
                                        let wa = mux(jt.bit(0), fa, j256 - fa);
                                        let wb = mux(jt.bit(1), fb, j256 - fb);
                                        let di = jt.bit(0).zext::<32>();
                                        let dj = jt.bit(1).zext::<32>();
                                        let ci = sra(ju, 8) + di;
                                        let cj = sra(jv, 8) + dj;
                                        let ws = self.tcs.get();
                                        let wt = self.tct.get();
                                        let wa17 = wa.resize::<17>();
                                        let wb17 = wb.resize::<17>();
                                        with!(self <= {
                                            mii: wrap(ci, self.mlw.get(), ws),
                                            mjj: wrap(cj, self.mlh.get(), wt),
                                            mw: wa17.mul::<17>(wb17),
                                        });
                                        DefaultClock::rising().await;
                                        let toff = texel_at(
                                            self.mii.get(),
                                            self.mjj.get(),
                                            self.mlw.get(),
                                        );
                                        let tan = self.mbase.get() + toff;
                                        let tln = tan.slice::<6, 6>();
                                        let twn = tan.slice::<2, 4>();
                                        with!(self <= {
                                            taddr: tan,
                                            tca: tln.concat::<4, 10>(twn),
                                        });
                                        DefaultClock::rising().await;
                                        let ta = self.taddr.get();
                                        let tline = ta.slice::<6, 6>();
                                        let tsh2 = tline.raw() as usize;
                                        let tvb = self.cvalid.get() >> tsh2;
                                        let tcav = self.tca.get();
                                        with!(self <= {
                                            ttag: self.ctag.read(tline),
                                            tok: tvb.bit(0),
                                            tdat: self.cdata.read(tcav),
                                        });
                                        DefaultClock::rising().await;
                                        // A miss reads the texel's block, one
                                        // burst of sixteen beats, into its
                                        // line, and keeps the texel's own word
                                        // as it passes.
                                        let tad = self.taddr.get();
                                        let ttop = tad.slice::<12, 20>();
                                        let ttg = self.ttag.get();
                                        let teq = Bit::from(ttg == ttop);
                                        let thit = self.tok.get() & teq;
                                        self.tmiss.set(!thit);
                                        if !thit.to_bool() {
                                            until(DefaultClock::rising, || {
                                                issue.ready().to_bool()
                                            })
                                            .await;
                                            let tad6 = self.taddr.get();
                                            let tb6 = tad6 >> 6usize;
                                            let tb0 = tb6 << 6usize;
                                            let tblock = tb0.slice::<0, A>();
                                            issue.send(Issue {
                                                read: Bit::One,
                                                addr: tblock,
                                                len: U::<8>::from(15u8),
                                                size: U::<3>::from(2u8),
                                                burst: BurstKind::Incr,
                                                lock: Bit::Zero,
                                                cache: U::<4>::from(0u8),
                                                prot: U::<3>::from(0u8),
                                                qos: U::<4>::from(0u8),
                                                region: U::<4>::from(0u8),
                                            });
                                            self.word.set(U::<5>::from(0u8));
                                            for _ in 0..16 {
                                                until(DefaultClock::rising, || {
                                                    landing(
                                                        rdata.peek().is_some(),
                                                        release.ready(),
                                                        done.peek().is_some(),
                                                    )
                                                    .to_bool()
                                                })
                                                .await;
                                                let fd = rdata.head().data;
                                                let fa = self.taddr.get();
                                                let fl = fa.slice::<6, 6>();
                                                let fwd = self.word.get();
                                                let fw = fwd.slice::<0, 4>();
                                                let fws = fa.slice::<2, 4>();
                                                let mine = Bit::from(fw == fws);
                                                let last = Bit::from(fw == 15);
                                                let fill = Bit::One;
                                                let fp = fl.concat::<4, 10>(fw);
                                                let fg = fa.slice::<12, 20>();
                                                let f1 = U::<64>::from(1u8);
                                                let fsh = fl.raw() as usize;
                                                let fcv = self.cvalid.get();
                                                let fv = fcv | (f1 << fsh);
                                                with!(self <= {
                                                    word: fwd + 1,
                                                    mine ? tfil: fd,
                                                    fill ? { cdata.at(fp): fd },
                                                    last ? { ctag.at(fl): fg },
                                                    last ? cvalid: fv,
                                                });
                                            }
                                        }
                                        DefaultClock::rising().await;
                                        // The texel's weight times each of
                                        // its channels, then its pass's
                                        // sums, which the pass's first
                                        // texel starts.
                                        let pt0 = self.tdat.get();
                                        let pt1 = self.tfil.get();
                                        let ptm = self.tmiss.get();
                                        let pt = mux(ptm, pt1, pt0);
                                        self.mpp.set(weigh(self.mw.get(), pt));
                                        DefaultClock::rising().await;
                                        let s2 = self.mpass.get();
                                        let s1 = !s2;
                                        let sf = Bit::from(self.mt.get() == 0);
                                        let sm1 = self.msum1.get();
                                        let sm2 = self.msum2.get();
                                        let sacc = mux(s2, sm2, sm1);
                                        let sp = self.mpp.get();
                                        let sum = add_terms(sf, sacc, sp);
                                        with!(self <= {
                                            mk: self.mk.get() + 1,
                                            s1 ? msum1: sum,
                                            s2 ? msum2: sum,
                                        });
                                        DefaultClock::rising().await;
                                    }
                                    // Each pass's sample; the two levels
                                    // blended by the level of detail's
                                    // fraction; and the environment, its
                                    // products and then their divide.
                                    if (self.tex_on.get() & self.hit.get())
                                        .to_bool()
                                    {
                                        with!(self <= {
                                            ms1: sum_texel(self.msum1.get()),
                                            ms2: sum_texel(self.msum2.get()),
                                            mnt: U::<4>::from(0u8),
                                        });
                                        DefaultClock::rising().await;
                                        let k1 = self.ms1.get();
                                        let k2 = self.ms2.get();
                                        let kf = self.mfr.get();
                                        self.mkk.set(mix_levels(k1, k2, kf));
                                        DefaultClock::rising().await;
                                        let xm = mixed_texel(self.mkk.get());
                                        let xs = self.ms1.get();
                                        let xt = mux(self.mtwo.get(), xm, xs);
                                        self.mtx.set(xt);
                                        DefaultClock::rising().await;
                                        // The fragment by the texel; the
                                        // fragment and the environment's
                                        // colour by the texel; and the
                                        // fragment and the texel by the
                                        // texel's alpha.
                                        let ef = self.rgb.get();
                                        let et = self.mtx.get();
                                        let ec = self.tenvc.get();
                                        let w32 = U::<32>::from(0xff_ffffu32);
                                        let nt = w32 - et;
                                        let ta8 = et.slice::<24, 8>();
                                        let ta16 = ta8.concat::<8, 16>(ta8);
                                        let ta24 = ta16.concat::<8, 24>(ta8);
                                        let ta32 = ta24.resize::<32>();
                                        let na32 = w32 - ta32;
                                        let z8 = U::<8>::from(0u8);
                                        let fa8 = ef.slice::<24, 8>();
                                        let fr8 = ef.slice::<16, 8>();
                                        let tr8 = et.slice::<16, 8>();
                                        let fg8 = ef.slice::<8, 8>();
                                        let tg8 = et.slice::<8, 8>();
                                        let fb8 = ef.slice::<0, 8>();
                                        let tb8 = et.slice::<0, 8>();
                                        with!(self <= {
                                            tma: blend_sum(fa8, z8, ta8, z8),
                                            tmr: blend_sum(fr8, z8, tr8, z8),
                                            tmg: blend_sum(fg8, z8, tg8, z8),
                                            tmb: blend_sum(fb8, z8, tb8, z8),
                                            mbl: sums3(ef, ec, nt, et),
                                            mdc: sums3(ef, et, na32, ta32),
                                        });
                                        DefaultClock::rising().await;
                                        let ea = over255(self.tma.get());
                                        let er = over255(self.tmr.get());
                                        let eg = over255(self.tmg.get());
                                        let eb = over255(self.tmb.get());
                                        let ear = ea.concat::<8, 16>(er);
                                        let earg = ear.concat::<8, 24>(eg);
                                        let em = earg.concat::<8, 32>(eb);
                                        let el = over255x3(self.mbl.get());
                                        let ed = over255x3(self.mdc.get());
                                        let f2 = self.rgb.get();
                                        let t2 = self.mtx.get();
                                        let ar = add255(
                                            f2.slice::<16, 8>(),
                                            t2.slice::<16, 8>(),
                                        );
                                        let ag = add255(
                                            f2.slice::<8, 8>(),
                                            t2.slice::<8, 8>(),
                                        );
                                        let ab = add255(
                                            f2.slice::<0, 8>(),
                                            t2.slice::<0, 8>(),
                                        );
                                        let ez = U::<8>::from(0u8);
                                        let ad = ez
                                            .concat::<8, 16>(ar)
                                            .concat::<8, 24>(ag)
                                            .concat::<8, 32>(ab);
                                        let tcl = self.tclass.get();
                                        let tev = self.tenv.get();
                                        let how = tcl.concat::<3, 6>(tev);
                                        let te0 = tex_env(
                                            f2, t2, em, el, ed, ad, how,
                                        );
                                        self.tcol.set(te0);
                                        DefaultClock::rising().await;
                                    }
                                    // Fog (issue 998), after the texture
                                    // and before the alpha test: each of
                                    // red, green and blue by the factor
                                    // and the fog's by what it leaves, a
                                    // turn before the read divides them,
                                    // as the blend's are. A fogged pixel
                                    // takes this one cycle more.
                                    if (self.son.get()
                                        & self.fon.get()
                                        & self.hit.get())
                                    .to_bool()
                                    {
                                        let fp = mux(
                                            self.tex_on.get(),
                                            self.tcol.get(),
                                            self.rgb.get(),
                                        );
                                        let ff = channel(self.fc.get());
                                        let nf = U::<8>::from(255u8) - ff;
                                        let fk = self.fcol.get();
                                        with!(self <= {
                                            fsr: blend_sum(
                                                fp.slice::<16, 8>(),
                                                fk.slice::<16, 8>(),
                                                ff,
                                                nf,
                                            ),
                                            fsg: blend_sum(
                                                fp.slice::<8, 8>(),
                                                fk.slice::<8, 8>(),
                                                ff,
                                                nf,
                                            ),
                                            fsb: blend_sum(
                                                fp.slice::<0, 8>(),
                                                fk.slice::<0, 8>(),
                                                ff,
                                                nf,
                                            ),
                                            fsa: fp.slice::<24, 8>(),
                                        });
                                        DefaultClock::rising().await;
                                    }
                                    // A pixel of an entry that tests
                                    // depth, or has the pixel's state
                                    // (issue 993), reads the depth and
                                    // the colour there first; is decided
                                    // a turn later, with the depth test,
                                    // the alpha test, the blend and the
                                    // mask; and is written a turn after
                                    // that at the same address. So the
                                    // depth bank has one port here, the
                                    // colour's copy one read, and no
                                    // write enable waits on a compare
                                    // (issue 992).
                                    if ((self.zon.get() | self.son.get())
                                        & self.hit.get())
                                    .to_bool()
                                    {
                                        let pa = self.pa.get();
                                        let fogged = self
                                            .fsa
                                            .get()
                                            .concat::<8, 16>(over255(
                                                self.fsr.get(),
                                            ))
                                            .concat::<8, 24>(over255(
                                                self.fsg.get(),
                                            ))
                                            .concat::<8, 32>(over255(
                                                self.fsb.get(),
                                            ));
                                        let plain = mux(
                                            self.tex_on.get(),
                                            self.tcol.get(),
                                            self.rgb.get(),
                                        );
                                        let fon = self.son.get() & self.fon.get();
                                        with!(self <= {
                                            dread: self.zbank.read(pa),
                                            dtag: self.zmark.read(pa),
                                            dcol: self.dbank.read(pa),
                                            zq: depth16(self.zc.get()),
                                            srcq: mux(fon, fogged, plain),
                                        });
                                        DefaultClock::rising().await;
                                        let src = self.srcq.get();
                                        // The depth and the stencil
                                        // there: the farthest and nought
                                        // where the tile has not written.
                                        let fresh =
                                            self.dtag.get() == self.serial.get();
                                        let dw = self.dread.get();
                                        let d16 = mux(
                                            fresh,
                                            dw.slice::<0, 16>(),
                                            U::<16>::from(0xffffu32),
                                        );
                                        let s8 = mux(
                                            fresh,
                                            dw.slice::<16, 8>(),
                                            U::<8>::from(0u8),
                                        );
                                        // Each test against both what is
                                        // there and what a fresh pixel
                                        // reads as, the mark choosing a
                                        // result rather than an input, so
                                        // that its compare runs beside the
                                        // comparators (issue 1534).
                                        let deep = mux(
                                            fresh,
                                            depth_pass(
                                                self.zfunc.get(),
                                                self.zq.get(),
                                                dw.slice::<0, 16>(),
                                            ),
                                            depth_pass(
                                                self.zfunc.get(),
                                                self.zq.get(),
                                                U::<16>::from(0xffffu32),
                                            ),
                                        );
                                        let alpha = depth_pass(
                                            self.afunc.get(),
                                            src.slice::<24, 8>().resize::<16>(),
                                            self.aref.get().resize::<16>(),
                                        );
                                        let son = self.son.get();
                                        // GL's order: the alpha test, the
                                        // stencil test, the depth test
                                        // (issue 998). The stencil takes
                                        // the operation for how the pixel
                                        // did, in the bits the write mask
                                        // holds, wherever the alpha test
                                        // passed; the depth is written
                                        // only where all three passed.
                                        let ston = son & self.sten.get();
                                        let sm = self.smask.get();
                                        let sr = self.sref.get();
                                        let sok = mux(
                                            fresh,
                                            depth_pass(
                                                self.sfunc.get(),
                                                (sr & sm).resize::<16>(),
                                                (dw.slice::<16, 8>() & sm)
                                                    .resize::<16>(),
                                            ),
                                            depth_pass(
                                                self.sfunc.get(),
                                                (sr & sm).resize::<16>(),
                                                U::<16>::from(0u8),
                                            ),
                                        );
                                        let apass =
                                            !(son & self.aon.get()) | alpha;
                                        let spass = !ston | sok;
                                        let dpass = !self.zon.get() | deep;
                                        // The stencil each of the three
                                        // operations would leave, in the
                                        // bits the write mask holds, side
                                        // by side, and the tests choose
                                        // one last: the depth test then
                                        // waits on a choice rather than on
                                        // the arithmetic (issue 1534).
                                        let swm = self.swmask.get();
                                        let on_fail = stencil_put(
                                            self.sfail.get(),
                                            s8,
                                            sr,
                                            ston,
                                            swm,
                                        );
                                        let on_zfail = stencil_put(
                                            self.szfail.get(),
                                            s8,
                                            sr,
                                            ston,
                                            swm,
                                        );
                                        let on_zpass = stencil_put(
                                            self.szpass.get(),
                                            s8,
                                            sr,
                                            ston,
                                            swm,
                                        );
                                        let sw = mux(
                                            sok,
                                            mux(dpass, on_zpass, on_zfail),
                                            on_fail,
                                        );
                                        let zw = self.zon.get()
                                            & self.zwrite.get()
                                            & apass
                                            & spass
                                            & dpass;
                                        // The blend's factors, a channel
                                        // at a time, as `model::factor`
                                        // has them: the first of the
                                        // blend's three turns (issue 993).
                                        let dst = self.dcol.get();
                                        let (sf, df) =
                                            (self.sfac.get(), self.dfac.get());
                                        let sa = src.slice::<24, 8>();
                                        let da = dst.slice::<24, 8>();
                                        let (sr, dr) = (
                                            src.slice::<16, 8>(),
                                            dst.slice::<16, 8>(),
                                        );
                                        let (sg, dg) = (
                                            src.slice::<8, 8>(),
                                            dst.slice::<8, 8>(),
                                        );
                                        let (sb, db) = (
                                            src.slice::<0, 8>(),
                                            dst.slice::<0, 8>(),
                                        );
                                        let (no, yes) = (Bit::Zero, Bit::One);
                                        let fs = factor(sf, sa, da, sa, da, yes)
                                            .concat::<8, 16>(factor(
                                                sf, sr, dr, sa, da, no,
                                            ))
                                            .concat::<8, 24>(factor(
                                                sf, sg, dg, sa, da, no,
                                            ))
                                            .concat::<8, 32>(factor(
                                                sf, sb, db, sa, da, no,
                                            ));
                                        let fd = factor(df, sa, da, sa, da, yes)
                                            .concat::<8, 16>(factor(
                                                df, sr, dr, sa, da, no,
                                            ))
                                            .concat::<8, 24>(factor(
                                                df, sg, dg, sa, da, no,
                                            ))
                                            .concat::<8, 32>(factor(
                                                df, sb, db, sa, da, no,
                                            ));
                                        with!(self <= {
                                            zpass: apass & spass & dpass,
                                            zsw: zw | (ston & apass),
                                            zsv: sw.concat::<16, 24>(mux(
                                                zw,
                                                self.zq.get(),
                                                d16,
                                            )),
                                            fsq: fs,
                                            fdq: fd,
                                        });
                                        DefaultClock::rising().await;
                                        // The second: each channel's two
                                        // products and their sum. The
                                        // third: the sum over 255, and the
                                        // mask. A pixel that only tests
                                        // depth needs neither.
                                        if son.to_bool() {
                                            let s = self.srcq.get();
                                            let d = self.dcol.get();
                                            let f = self.fsq.get();
                                            let g = self.fdq.get();
                                            with!(self <= {
                                                sum_a: blend_sum(
                                                    s.slice::<24, 8>(),
                                                    d.slice::<24, 8>(),
                                                    f.slice::<24, 8>(),
                                                    g.slice::<24, 8>(),
                                                ),
                                                sum_r: blend_sum(
                                                    s.slice::<16, 8>(),
                                                    d.slice::<16, 8>(),
                                                    f.slice::<16, 8>(),
                                                    g.slice::<16, 8>(),
                                                ),
                                                sum_g: blend_sum(
                                                    s.slice::<8, 8>(),
                                                    d.slice::<8, 8>(),
                                                    f.slice::<8, 8>(),
                                                    g.slice::<8, 8>(),
                                                ),
                                                sum_b: blend_sum(
                                                    s.slice::<0, 8>(),
                                                    d.slice::<0, 8>(),
                                                    f.slice::<0, 8>(),
                                                    g.slice::<0, 8>(),
                                                ),
                                                lres: logic_op(
                                                    self.lop.get(),
                                                    s,
                                                    d,
                                                ),
                                            });
                                            DefaultClock::rising().await;
                                            let top = over255(self.sum_a.get());
                                            let mixed = top
                                                .concat::<8, 16>(over255(
                                                    self.sum_r.get(),
                                                ))
                                                .concat::<8, 24>(over255(
                                                    self.sum_g.get(),
                                                ))
                                                .concat::<8, 32>(over255(
                                                    self.sum_b.get(),
                                                ));
                                            let bl = mux(
                                                self.bon.get(),
                                                mixed,
                                                self.srcq.get(),
                                            );
                                            // A logic operation is in place
                                            // of the blend (issue 998).
                                            let lr = self.lres.get();
                                            let lon = self.lon.get();
                                            let lg = mux(lon, lr, bl);
                                            self.bout.set(masked(
                                                lg,
                                                self.dcol.get(),
                                                self.cmask.get(),
                                            ));
                                            DefaultClock::rising().await;
                                        }
                                    }
                                    let px = self.x.get();
                                    let py = self.y.get();
                                    // The pixel's word. The address is
                                    // worked out at the link's width
                                    // from the start, not at the sixteen
                                    // bits the walk is counted in: on
                                    // the board's rows of 1024 a pixel's
                                    // offset passes sixteen bits at row
                                    // 16, and an offset formed there and
                                    // widened after wrapped every later
                                    // row into the first sixteen (issue
                                    // 1178).
                                    let addr = (((py.resize::<A>() << LOGW)
                                        + px.resize::<A>())
                                        << WORD)
                                        + U::<A>::from(BASE as u32);
                                    // A run of the row's pixels goes out
                                    // as one burst (issue 987). A pixel
                                    // in the primitive with no burst
                                    // under way starts one, as long as
                                    // the pixels left in the row, at
                                    // most sixteen, and short of the
                                    // next 4 KiB page; every pixel it
                                    // covers takes a beat, its strobes
                                    // on where the pixel is in the
                                    // primitive and off where it is not.
                                    // A triangle's pixels in a row are
                                    // one run, so a burst wastes beats
                                    // only past the run's end.
                                    let on = self.beats.get() != 0;
                                    let start = !on
                                        & (self.hit.get() & !self.tiled.get())
                                            .to_bool();
                                    // In a tile, the pixel and its mark
                                    // into the bank at `{y, x}` within it.
                                    let at = py
                                        .slice::<0, 6>()
                                        .concat::<6, 12>(px.slice::<0, 6>());
                                    // A pixel that tests depth or alpha is
                                    // kept where it passed, and writes its
                                    // depth if the entry says so, and its
                                    // colour, blended and masked, unless
                                    // the mask holds no channel. A load
                                    // writes the framebuffer's pixel.
                                    // Each mark written takes the tile's
                                    // serial, or nought in a scrub, which
                                    // writes every mark and nothing else.
                                    let (zon, son) =
                                        (self.zon.get(), self.son.get());
                                    let pass = !(zon | son) | self.zpass.get();
                                    let scrub = self.scrub.get();
                                    let load = self.load.get();
                                    let hit = self.tiled.get() & self.hit.get();
                                    let keep = hit & pass & !scrub & !load;
                                    let none = son
                                        & Bit::from(self.cmask.get() == 0);
                                    let ckeep = (keep & !none) | load;
                                    // The depth and the stencil, as the
                                    // decide turn has them (issue 998).
                                    let zkeep = hit
                                        & (zon | son)
                                        & self.zsw.get()
                                        & !scrub
                                        & !load;
                                    let tag = mux(
                                        scrub,
                                        U::<8>::from(0u8),
                                        self.serial.get(),
                                    );
                                    let word = mux(
                                        load,
                                        rdata.head().data,
                                        mux(
                                            son,
                                            self.bout.get(),
                                            mux(
                                                self.tex_on.get(),
                                                self.tcol.get(),
                                                self.rgb.get(),
                                            ),
                                        ),
                                    );
                                    let (mkeep, zmkeep) =
                                        (ckeep | scrub, zkeep | scrub);
                                    let pa = self.pa.get();
                                    with!(self <= {
                                        ckeep ? { bank.at(at): word },
                                        ckeep ? { dbank.at(at): word },
                                        mkeep ? { mark.at(at): tag },
                                        zkeep ? { zbank.at(pa): self.zsv.get() },
                                        zmkeep ? { zmark.at(pa): tag },
                                    });
                                    let rest =
                                        (self.xb.get() - px).resize::<32>();
                                    let page = U::<32>::from(PAGE)
                                        - (addr.resize::<32>() >> WORD)
                                            .slice::<0, 10>()
                                            .resize::<32>();
                                    let room = mux(rest < page, rest, page);
                                    let run = U::<32>::from(RUN);
                                    let len = mux(room < run, room, run)
                                        .resize::<8>();
                                    if start {
                                        issue.send(Issue {
                                            read: Bit::Zero,
                                            addr,
                                            len,
                                            size: U::<3>::from(2u8),
                                            burst: BurstKind::Incr,
                                            lock: Bit::Zero,
                                            cache: U::<4>::from(0u8),
                                            prot: U::<3>::from(0u8),
                                            qos: U::<4>::from(0u8),
                                            region: U::<4>::from(0u8),
                                        });
                                        self.issued.set(self.issued.get() + 1);
                                    }
                                    let owed = mux(on, self.beats.get(), len);
                                    if on | start {
                                        wbeat.send(W {
                                            data: self.rgb.get(),
                                            strb: mux(
                                                self.hit.get(),
                                                U::<4>::from(15u8),
                                                U::<4>::from(0u8),
                                            ),
                                            last: Bit::from(mux(
                                                on,
                                                owed == 1,
                                                owed == 0,
                                            )),
                                        });
                                    }
                                    // The column advances, each edge
                                    // and each channel takes its column
                                    // step, and a burst under way owes
                                    // one beat fewer.
                                    let nu = self.tnuy.get();
                                    let nud = self.tnuyd.get();
                                    let nv = self.tnvy.get();
                                    let nvd = self.tnvyd.get();
                                    let (nuy, nvy) = (nu + nud, nv + nvd);
                                    with!(self <= {
                                        beats: mux(
                                            on,
                                            self.beats.get() - 1,
                                            mux(start, len, self.beats.get()),
                                        ),
                                        x: px + 1,
                                        e0: self.e0.get() + self.d0x.get(),
                                        e1: self.e1.get() + self.d1x.get(),
                                        e2: self.e2.get() + self.d2x.get(),
                                        cr: self.cr.get() + self.crx.get(),
                                        cg: self.cg.get() + self.cgx.get(),
                                        cb: self.cb.get() + self.cbx.get(),
                                        zc: self.zc.get() + self.zdx.get(),
                                        fc: self.fc.get() + self.fdx.get(),
                                        pac: self.pac.get() + self.padx.get(),
                                        tuc: self.tuc.get() + self.tudx.get(),
                                        tvc: self.tvc.get() + self.tvdx.get(),
                                        tqc: self.tqc.get() + self.tqdx.get(),
                                        tnuy: nuy,
                                        tnvy: nvy,
                                        pa: py.slice::<0, 6>().concat::<6, 12>(
                                            (px + 1).slice::<0, 6>(),
                                        ),
                                    });
                                }
                                // The next row: the column goes back to
                                // the first and each edge and channel is
                                // reloaded from its row value plus its
                                // row step.
                                let q0 = self.r0.get() + self.d0y.get();
                                let q1 = self.r1.get() + self.d1y.get();
                                let q2 = self.r2.get() + self.d2y.get();
                                let qr = self.lr.get() + self.cry.get();
                                let qg = self.lg.get() + self.cgy.get();
                                let qb = self.lb.get() + self.cby.get();
                                let qz = self.zr.get() + self.zdy.get();
                                let qf = self.fr.get() + self.fdy.get();
                                let qa = self.par.get() + self.pady.get();
                                let qu = self.tur.get() + self.tudy.get();
                                let qv = self.tvr.get() + self.tvdy.get();
                                let qq = self.tqr.get() + self.tqdy.get();
                                with!(self <= {
                                    x: self.xa.get(),
                                    y: self.y.get() + 1,
                                    zc: qz, zr: qz,
                                    fc: qf, fr: qf,
                                    pac: qa, par: qa,
                                    tuc: qu, tur: qu,
                                    tvc: qv, tvr: qv,
                                    tqc: qq, tqr: qq,
                                    tnux: self.tnux.get() + self.tnuxd.get(),
                                    tnvx: self.tnvx.get() + self.tnvxd.get(),
                                    tnuy: self.tnuy0.get(),
                                    tnvy: self.tnvy0.get(),
                                    pa: (self.y.get() + 1)
                                        .slice::<0, 6>()
                                        .concat::<6, 12>(
                                            self.xa.get().slice::<0, 6>(),
                                        ),
                                    e0: q0, r0: q0,
                                    e1: q1, r1: q1,
                                    e2: q2, r2: q2,
                                    cr: qr, lr: qr,
                                    cg: qg, lg: qg,
                                    cb: qb, lb: qb,
                                });
                            }
                            // A scrub was not an entry of the list, and
                            // leaves every mark nought, so the serial
                            // starts again from one; a load comes after
                            // it, and was not an entry of the list either.
                            let (scrub, load) =
                                (self.scrub.get(), self.load.get());
                            with!(self <= {
                                insn: mux(
                                    scrub | load,
                                    self.insn.get(),
                                    self.insn.get() + 1,
                                ),
                                serial: mux(
                                    scrub,
                                    U::<8>::from(1u8),
                                    self.serial.get(),
                                ),
                                clean: self.clean.get() | scrub,
                                scrub: Bit::Zero,
                                load: load & scrub,
                            });
                        }
                        // A tile drawn goes out a row at a time, each one
                        // burst of 64 beats, its strobes on where a pixel
                        // was written and off where none was, so a pixel
                        // the tile's entries did not cover keeps what
                        // memory had, as it does from a flat list. A
                        // pixel was written when its mark is the tile's
                        // serial, so the next tile, with the next serial,
                        // finds the bank empty without a mark cleared.
                        if self.tiled.get().to_bool() {
                            self.wr.set(U::<6>::from(0u8));
                            for _ in 0..self.th.get().raw() as usize {
                                DefaultClock::rising().await;
                                self.wc.set(U::<7>::from(0u8));
                                // Sixty-five turns a row. Each reads the
                                // word and the mark at one column. The
                                // first turn sends the burst and the rest
                                // each send the word read the turn before,
                                // so the read lands in a register, and
                                // each memory has one read here and the
                                // walk's write as its other port.
                                for _ in 0..=TILE as usize {
                                    until(DefaultClock::rising, || {
                                        ((Bit::from(self.wc.get() == 0)
                                            & issue.ready())
                                            | (Bit::from(self.wc.get() != 0)
                                                & wbeat.ready()))
                                        .to_bool()
                                    })
                                    .await;
                                    let c = self.wc.get();
                                    let at = self
                                        .wr
                                        .get()
                                        .concat::<6, 12>(c.slice::<0, 6>());
                                    let row = (self.oy.get()
                                        + self.wr.get().resize::<16>())
                                    .resize::<A>();
                                    if c == 0 {
                                        issue.send(Issue {
                                            read: Bit::Zero,
                                            addr: (((row << LOGW)
                                                + self.ox.get().resize::<A>())
                                                << WORD)
                                                + U::<A>::from(BASE as u32),
                                            len: U::<8>::from(TILE_LEN),
                                            size: U::<3>::from(2u8),
                                            burst: BurstKind::Incr,
                                            lock: Bit::Zero,
                                            cache: U::<4>::from(0u8),
                                            prot: U::<3>::from(0u8),
                                            qos: U::<4>::from(0u8),
                                            region: U::<4>::from(0u8),
                                        });
                                        self.issued.set(self.issued.get() + 1);
                                    }
                                    if c != 0 {
                                        wbeat.send(W {
                                            data: self.rd.get(),
                                            strb: mux(
                                                self.rm.get()
                                                    == self.serial.get(),
                                                U::<4>::from(15u8),
                                                U::<4>::from(0u8),
                                            ),
                                            last: Bit::from(
                                                c == U::<7>::from(TILE),
                                            ),
                                        });
                                    }
                                    with!(self <= {
                                        rd: self.bank.read(at),
                                        rm: self.mark.read(at),
                                        wc: c + 1,
                                    });
                                }
                                self.wr.set(self.wr.get() + 1);
                            }
                            // The next tile's serial. Past 255 the marks
                            // are scrubbed before it draws.
                            let s = self.serial.get();
                            with!(self <= {
                                serial: s + 1,
                                clean: self.clean.get() & Bit::from(s != 255),
                            });
                        }
                      }
                        // The list is drawn. Once every write it made
                        // has been answered, the count goes back to
                        // zero: that is how a program learns the list
                        // is done and may write the next, and `idle`
                        // rises with it. The zero is answered before
                        // the count is read again, so a memory that
                        // reorders a read past a write cannot hand back
                        // the old count and have the list drawn twice.
                        until(DefaultClock::rising, || {
                            (Bit::from(self.inflight.get() == 0)
                                & issue.ready()
                                & wbeat.ready())
                            .to_bool()
                        })
                        .await;
                        issue.send(Issue {
                            read: Bit::Zero,
                            addr: U::<A>::from(CTRL as u32),
                            len: U::<8>::from(0u8),
                            size: U::<3>::from(2u8),
                            burst: BurstKind::Incr,
                            lock: Bit::Zero,
                            cache: U::<4>::from(0u8),
                            prot: U::<3>::from(0u8),
                            qos: U::<4>::from(0u8),
                            region: U::<4>::from(0u8),
                        });
                        wbeat.send(W {
                            data: U::<32>::from(0u8),
                            strb: U::<4>::from(15u8),
                            last: Bit::One,
                        });
                        with!(self <= {
                            issued: self.issued.get() + 1,
                            finished: Bit::One,
                        });
                        DefaultClock::rising().await;
                        until(DefaultClock::rising, || {
                            Bit::from(self.inflight.get() == 0).to_bool()
                        })
                        .await;
                    }
                }
            },
        )
        .await;
    }
}
// end{run}

#[cfg(test)]
mod tests {
    use super::*;

    /// The decide stage's inputs: the mark's verdict, the depth and
    /// stencil read, the pixel's depth, the depth and stencil
    /// comparisons, the reference and its mask, the operations on a
    /// stencil failure, a depth failure and a pass, whether the stencil
    /// is on, and its write mask.
    type Inputs = (
        Bit,
        U<24>,
        U<16>,
        U<3>,
        U<3>,
        U<8>,
        U<8>,
        [U<3>; 3],
        Bit,
        U<8>,
    );

    /// The decide stage's depth and stencil, as they were before issue
    /// 1534 split them: the mark choosing the depth and the stencil
    /// read, then the tests, then the stencil's operation, then its
    /// arithmetic. The depth test's result, the stencil test's, and the
    /// stencil written.
    fn before(a: Inputs) -> (Bit, Bit, U<8>) {
        let (fresh, dw, zq, zfunc, sfunc, sr, sm, ops, ston, swm) = a;
        let d16 = mux(fresh, dw.slice::<0, 16>(), U::<16>::from(0xffffu32));
        let s8 = mux(fresh, dw.slice::<16, 8>(), U::<8>::from(0u8));
        let deep = depth_pass(zfunc, zq, d16);
        let sok = depth_pass(
            sfunc,
            (sr & sm).resize::<16>(),
            (s8 & sm).resize::<16>(),
        );
        let sop = mux(sok, mux(deep, ops[2], ops[1]), ops[0]);
        let sn = stencil_step(sop, s8, sr);
        (deep, sok, mux(ston, (s8 & !swm) | (sn & swm), s8))
    }

    /// The same, as the decide stage now has it (issue 1534).
    fn after(a: Inputs) -> (Bit, Bit, U<8>) {
        let (fresh, dw, zq, zfunc, sfunc, sr, sm, ops, ston, swm) = a;
        let s8 = mux(fresh, dw.slice::<16, 8>(), U::<8>::from(0u8));
        let deep = mux(
            fresh,
            depth_pass(zfunc, zq, dw.slice::<0, 16>()),
            depth_pass(zfunc, zq, U::<16>::from(0xffffu32)),
        );
        let sok = mux(
            fresh,
            depth_pass(
                sfunc,
                (sr & sm).resize::<16>(),
                (dw.slice::<16, 8>() & sm).resize::<16>(),
            ),
            depth_pass(sfunc, (sr & sm).resize::<16>(), U::<16>::from(0u8)),
        );
        let put = |op| stencil_put(op, s8, sr, ston, swm);
        let sw = mux(sok, mux(deep, put(ops[2]), put(ops[1])), put(ops[0]));
        (deep, sok, sw)
    }

    /// Issue 1534's restructure decides every pixel as before: a million
    /// draws of the stage's inputs, the depth and the stencil read and
    /// the pixel's depth from values near the edges as often as not, and
    /// every comparison and operation, fresh and written.
    #[test]
    fn the_restructured_decide_is_the_same_function() {
        let mut x = 0x2545_f491_4f6c_dd1du64;
        let mut next = || {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x
        };
        let edge = |r: u64, v: u64, top: u64| match r % 4 {
            0 => 0,
            1 => top,
            2 => top - 1,
            _ => v & top,
        };
        for _ in 0..1_000_000 {
            let r = next();
            let fresh = Bit::from(r & 1 == 1);
            let d = edge(r >> 1, next(), 0xffff);
            let s = edge(r >> 3, next(), 0xff);
            let dw = U::<24>::from((s << 16 | d) as u32);
            let zq = U::<16>::from(edge(r >> 5, next(), 0xffff) as u32);
            let f3 = |v: u64| U::<3>::from((v & 7) as u32);
            let (zfunc, sfunc) = (f3(r >> 7), f3(r >> 10));
            let ops = [f3(r >> 13), f3(r >> 16), f3(r >> 19)];
            let b8 = |v: u64| U::<8>::from((v & 0xff) as u32);
            let (sr, sm, swm) = (b8(r >> 22), b8(r >> 30), b8(r >> 38));
            let ston = Bit::from(r >> 46 & 1 == 1);
            let a = (fresh, dw, zq, zfunc, sfunc, sr, sm, ops, ston, swm);
            let (was, now) = (before(a), after(a));
            assert_eq!(
                (was.0.to_bool(), was.1.to_bool(), was.2.raw()),
                (now.0.to_bool(), now.1.to_bool(), now.2.raw()),
                "{a:?}"
            );
        }
    }

    /// The largest of four numerators' magnitudes as `biggest` took it
    /// before #1565's timing: each pair's larger, then the larger of
    /// those.
    fn biggest_pairs(a: U<32>, b: U<32>, c: U<32>, d: U<32>) -> U<32> {
        let z = U::<32>::from(0u8);
        let m = |v: U<32>| mux(v.bit(31), z - v, v);
        let (ma, mb, mc, md) = (m(a), m(b), m(c), m(d));
        let ab = mux(ma < mb, mb, ma);
        let cd = mux(mc < md, md, mc);
        mux(ab < cd, cd, ab)
    }

    /// `biggest`'s comparisons side by side give the largest the pairs
    /// gave: a million draws of four numerators, each nought, one, the
    /// most negative, a value of either sign, or equal to another, as
    /// often as not.
    #[test]
    fn the_largest_numerator_is_as_before() {
        let mut x = 0x9e37_79b9_7f4a_7c15u64;
        let mut next = || {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x
        };
        for _ in 0..1_000_000 {
            let r = next();
            let mut v = [0u32; 4];
            for k in 0..4 {
                v[k] = match (r >> (3 * k)) & 7 {
                    0 => 0,
                    1 => 1,
                    2 => 0x8000_0000,
                    3 => 0xffff_ffff,
                    4 if k > 0 => v[k - 1],
                    5 if k > 0 => v[k - 1].wrapping_neg(),
                    _ => next() as u32,
                };
            }
            let u = v.map(U::<32>::from);
            assert_eq!(
                biggest(u[0], u[1], u[2], u[3]).raw(),
                biggest_pairs(u[0], u[1], u[2], u[3]).raw(),
                "{v:x?}"
            );
        }
    }
}
