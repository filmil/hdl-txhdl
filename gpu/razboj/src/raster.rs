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
use txhdl::types::{Bit, U};
use txhdl::{lower, with, Trace};
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
    /// The depth bank, a tile's depths, read and written at one address
    /// only, so that it is a block RAM of one port; a mark for each
    /// depth, the serial of the tile that wrote it as with the colour,
    /// so that a depth not written in this tile reads as the farthest;
    /// the depth and its mark read for the pixel under the walk, and the
    /// pixel's own depth; and whether the pixel passed, decided a turn
    /// after the read so that the banks' write enables come from a
    /// register.
    pub zbank: Mem<U<16>, 4096>,
    pub zmark: Mem<U<8>, 4096>,
    pub dread: Reg<U<16>>,
    pub dtag: Reg<U<8>>,
    pub zq: Reg<U<16>>,
    pub zpass: Reg<Bit>,
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
    /// The tile's colour again, written where and as the bank is and
    /// read only by the walk, for the colour already at a pixel, so that
    /// each of the two is a block RAM of one write and one read; the
    /// colour read there; the pixel's own, held a turn; and the colour
    /// the pixel writes, blended and masked.
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
    pub sum_a: Reg<U<17>>,
    pub sum_r: Reg<U<17>>,
    pub sum_g: Reg<U<17>>,
    pub sum_b: Reg<U<17>>,
    /// Whether the tile is loaded from the framebuffer before its
    /// entries, as one more entry after the scrub (issue 993).
    pub load: Reg<Bit>,
    /// Whether the entry is textured, so that its two texture slots are
    /// passed over (issue 997).
    pub texd: Reg<Bit>,
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

/// The bytes of `new` that `mask` holds, a bit a byte, over `old`.
#[lower]
fn masked(new: U<32>, old: U<32>, mask: U<4>) -> U<32> {
    let b = mux(mask.bit(0), new.slice::<0, 8>(), old.slice::<0, 8>());
    let g = mux(mask.bit(1), new.slice::<8, 8>(), old.slice::<8, 8>());
    let r = mux(mask.bit(2), new.slice::<16, 8>(), old.slice::<16, 8>());
    let a = mux(mask.bit(3), new.slice::<24, 8>(), old.slice::<24, 8>());
    a.concat::<8, 16>(r).concat::<8, 24>(g).concat::<8, 32>(b)
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
                    self.rgb.set(self.alpha.get().concat::<24, 32>(rgb));
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
                                          len: U::<8>::from(4u8),
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
                                  }
                                  // A textured entry's two slots more are
                                  // passed over: this rasteriser draws it
                                  // untextured (issue 997).
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
                                        with!(self <= {
                                            dread: self.zbank.read(pa),
                                            dtag: self.zmark.read(pa),
                                            dcol: self.dbank.read(pa),
                                            zq: depth16(self.zc.get()),
                                            srcq: self.rgb.get(),
                                        });
                                        DefaultClock::rising().await;
                                        let src = self.srcq.get();
                                        let deep = depth_pass(
                                            self.zfunc.get(),
                                            self.zq.get(),
                                            mux(
                                                self.dtag.get()
                                                    == self.serial.get(),
                                                self.dread.get(),
                                                U::<16>::from(0xffffu32),
                                            ),
                                        );
                                        let alpha = depth_pass(
                                            self.afunc.get(),
                                            src.slice::<24, 8>().resize::<16>(),
                                            self.aref.get().resize::<16>(),
                                        );
                                        let son = self.son.get();
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
                                            zpass: (!self.zon.get() | deep)
                                                & (!(son & self.aon.get())
                                                    | alpha),
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
                                            self.bout.set(masked(
                                                mux(
                                                    self.bon.get(),
                                                    mixed,
                                                    self.srcq.get(),
                                                ),
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
                                    let zkeep = keep & zon & self.zwrite.get();
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
                                            self.rgb.get(),
                                        ),
                                    );
                                    let (mkeep, zmkeep) =
                                        (ckeep | scrub, zkeep | scrub);
                                    let pa = self.pa.get();
                                    with!(self <= {
                                        ckeep ? { bank.at(at): word },
                                        ckeep ? { dbank.at(at): word },
                                        mkeep ? { mark.at(at): tag },
                                        zkeep ? { zbank.at(pa): self.zq.get() },
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
                                with!(self <= {
                                    x: self.xa.get(),
                                    y: self.y.get() + 1,
                                    zc: qz, zr: qz,
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
