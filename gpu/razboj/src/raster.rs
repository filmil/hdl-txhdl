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
    /// The tile buffer: a tile's colour, and a mark for each pixel
    /// written, at `{y, x}`, the low six bits of each coordinate.
    pub bank: Mem<U<32>, 4096>,
    pub mark: Mem<U<1>, 4096>,
    /// The write-out: the tile's rows on the screen, the row and the
    /// column going out, and the word and the mark read a cycle ahead
    /// for the next beat, so that the bank's read lands in a register,
    /// which is what makes it a block RAM.
    pub th: Reg<U<8>>,
    pub wr: Reg<U<6>>,
    pub wc: Reg<U<7>>,
    pub rd: Reg<U<32>>,
    pub rm: Reg<U<1>>,
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
                            with!(self <= {
                                insn: w0.slice::<0, 16>(),
                                n: w0.slice::<16, 16>(),
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
                            with!(self <= {
                                ox: w1.slice::<0, 10>().resize::<16>(),
                                oy: oy,
                                th: rows.resize::<8>(),
                                tile: self.tile.get() + 1,
                            });
                            DefaultClock::rising().await;
                        }
                        for _ in 0..self.n.get().raw() as usize {
                            // An edge for the instruction's index to
                            // read back, and for the sequence to
                            // begin a turn with.
                            DefaultClock::rising().await;
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
                                    + (self.insn.get().resize::<A>() << SHIFT),
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
                                let clearing = self.skind.get() == Kind::Clear;
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
                                // Every entry's alpha, last.
                                if self.word.get() == 15 {
                                    self.alpha.set(v.slice::<0, 8>());
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
                                for _ in self.xa.get().raw() as usize
                                    ..=self.xb.get().raw() as usize
                                {
                                    // A burst under way needs room for
                                    // its next beat; a pixel that would
                                    // start one, room for the burst and
                                    // its first beat.
                                    // In a tile the pixel goes to the
                                    // bank, which takes one a cycle.
                                    until(DefaultClock::rising, || {
                                        (self.tiled.get()
                                            | (Bit::from(self.beats.get() != 0)
                                                & wbeat.ready())
                                            | (Bit::from(
                                                self.beats.get() == 0,
                                            ) & (!self.hit.get()
                                                | (issue.ready()
                                                    & wbeat.ready()))))
                                        .to_bool()
                                    })
                                    .await;
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
                                    let keep = self.tiled.get() & self.hit.get();
                                    with!(self <= {
                                        keep ? {
                                            bank.at(at): self.rgb.get(),
                                            mark.at(at): U::<1>::from(1u8),
                                        },
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
                                with!(self <= {
                                    x: self.xa.get(),
                                    y: self.y.get() + 1,
                                    e0: q0, r0: q0,
                                    e1: q1, r1: q1,
                                    e2: q2, r2: q2,
                                    cr: qr, lr: qr,
                                    cg: qg, lg: qg,
                                    cb: qb, lb: qb,
                                });
                            }
                            self.insn.set(self.insn.get() + 1);
                        }
                        // A tile drawn goes out a row at a time, each one
                        // burst of 64 beats, its strobes on where a pixel
                        // was written and off where none was, so a pixel
                        // the tile's entries did not cover keeps what
                        // memory had, as it does from a flat list. Each
                        // mark is cleared as it is read, which leaves the
                        // bank empty for the next tile.
                        if self.tiled.get().to_bool() {
                            self.wr.set(U::<6>::from(0u8));
                            for _ in 0..self.th.get().raw() as usize {
                                DefaultClock::rising().await;
                                self.wc.set(U::<7>::from(0u8));
                                // Sixty-five turns a row. Each reads the
                                // word and the mark at one column, and
                                // clears that mark, at the same address,
                                // which a block RAM's port does in one
                                // cycle, reading first. The first turn
                                // sends the burst and the rest each send
                                // the word read the turn before, so the
                                // read lands in a register, and each
                                // memory has one port here and the walk's
                                // write as its other.
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
                                                self.rm.get() == 1,
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
                                        mark.at(at): U::<1>::from(0u8),
                                    });
                                }
                                self.wr.set(self.wr.get() + 1);
                            }
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
