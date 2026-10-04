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
//! it is not zero, then the fifteen words of each instruction at `DL`,
//! as one read burst of fifteen beats, then walks what they say. The
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
//! It is an AXI host, and it writes as a host client writes: a burst
//! of one beat per pixel, issued on `issue` with its beat on `wbeat`
//! in the same cycle, with the identifier the tracker granted handed
//! back on `release` when the write response arrives. Several writes
//! are in flight, as many as the tracker has identifiers. A read's
//! identifier is handed back when its beat arrives on `rdata`.
//!
//! The framebuffer's first word is at `BASE` and a pixel is one word,
//! so a pixel's address is `BASE + ((y << LOGW) + x) * 4`.
use txhdl::comp::{
    join2, mux, until, Clock, DefaultClock, Out, Reg, Rx, Tx, Unit, Wire,
};
use txhdl::types::{Bit, U};
use txhdl::{lower, with, Trace};
use txhdl_parts::bus::axi::{BurstKind, Done, Grant, Issue, R, W};

use crate::op::Kind;

/// A pixel is one word, so a pixel's byte address is its index in the
/// framebuffer shifted by this, and then offset by the framebuffer's
/// own base. A word of the display list is addressed the same way.
const WORD: usize = 2;

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
    /// The word this pixel takes, its colour, which for a shaded
    /// triangle is worked out from its three channels here.
    pub rgb: Wire<U<32>>,
    /// Instructions in the list, once the count has been read.
    pub left: Reg<U<16>>,
    /// Which instruction is being fetched, and which of its words.
    pub insn: Reg<U<16>>,
    pub word: Reg<U<4>>,
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
        (grant, done, rdata): (Rx<Grant<I>>, Rx<Done<I>>, Rx<R<32, I>>),
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
                    // The colour this pixel takes: a shaded triangle's
                    // three planes here, and every other entry's own.
                    let shade = channel(self.cr.get())
                        .concat::<8, 16>(channel(self.cg.get()))
                        .concat::<8, 24>(channel(self.cb.get()));
                    let shaded = self.kind.get() == Kind::Shaded;
                    let rgb = mux(shaded, shade, self.colour.get());
                    self.rgb.set(rgb.resize::<32>());
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
                    // means poll again. A read is issued once the link
                    // has room for it.
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
                    let count = rdata.head().data.slice::<0, 16>();
                    with!(self <= {
                        left: count,
                        insn: U::<16>::from(0u8),
                        finished:
                            mux(count == 0, self.finished.get(), Bit::Zero),
                    });
                    DefaultClock::rising().await;
                    if self.left.get() != 0 {
                        for _ in 0..self.left.get().raw() as usize {
                            // An edge for the instruction's index to
                            // read back, and for the sequence to
                            // begin a turn with.
                            DefaultClock::rising().await;
                            self.word.set(U::<4>::from(0u8));
                            // The instruction's fifteen words, as one
                            // read burst of fifteen beats, each latched
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
                                    + (self.insn.get().resize::<A>() << SHIFT),
                                len: U::<8>::from(14u8),
                                size: U::<3>::from(2u8),
                                burst: BurstKind::Incr,
                                lock: Bit::Zero,
                                cache: U::<4>::from(0u8),
                                prot: U::<3>::from(0u8),
                                qos: U::<4>::from(0u8),
                                region: U::<4>::from(0u8),
                            });
                            for _ in 0..15 {
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
                                    until(DefaultClock::rising, || {
                                        (!self.hit.get()
                                            | (issue.ready() & wbeat.ready()))
                                        .to_bool()
                                    })
                                    .await;
                                    let px = self.x.get();
                                    let py = self.y.get();
                                    // One pixel, as a burst of one
                                    // beat at the pixel's word. The
                                    // address is worked out at the
                                    // link's width, not at the sixteen
                                    // bits the walk is counted in: the
                                    // offset into the framebuffer fits
                                    // in sixteen, but the framebuffer's
                                    // own base need not, and an address
                                    // that wrapped would land on
                                    // whatever else the map has there.
                                    let pixel_at = (((py << LOGW) + px)
                                        << WORD)
                                        .resize::<A>()
                                        + U::<A>::from(BASE as u32);
                                    if self.hit.get().to_bool() {
                                        // `pixel_at` and not `addr`:
                                        // the lowering took the field
                                        // `addr:` of the write after
                                        // the walk for a use of a
                                        // local of that name (issue
                                        // 1024).
                                        issue.send(Issue {
                                            read: Bit::Zero,
                                            addr: pixel_at,
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
                                            data: self.rgb.get(),
                                            strb: U::<4>::from(15u8),
                                            last: Bit::One,
                                        });
                                        self.issued.set(self.issued.get() + 1);
                                    }
                                    // The column advances and each edge
                                    // and each channel takes its column
                                    // step.
                                    with!(self <= {
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
