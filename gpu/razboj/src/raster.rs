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
//! done in the cycle the entry is taken. This is what makes the
//! per-pixel work three adds and three sign tests.
//!
//! It finds its own work. The display list is in memory, in the
//! format `crate::dl` states, and the rasteriser reads it over the
//! same link it writes pixels on: it reads the count at `CTRL` until
//! it is not zero, then the six words of each instruction at `DL`,
//! one read at a time, then walks what they say. That front end is
//! written as the sequence it is, and a second process takes the
//! link's answers every cycle.
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
const SHIFT: usize = 5;

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
    /// Instructions in the list, once the count has been read.
    pub left: Reg<U<8>>,
    /// Which instruction is being fetched, and which of its words.
    pub insn: Reg<U<8>>,
    pub word: Reg<U<3>>,
    /// The list is drawn.
    pub finished: Reg<Bit>,
    /// The instruction being assembled, word by word. The third
    /// vertex is not here: its word is the last one, so it is read
    /// straight off the bus in the cycle the walk is set up.
    pub skind: Reg<Kind>,
    pub scol: Reg<U<24>>,
    pub sx0: Reg<U<10>>,
    pub sy0: Reg<U<10>>,
    pub sx1: Reg<U<10>>,
    pub sy1: Reg<U<10>>,
    pub sax: Reg<U<12>>,
    pub say: Reg<U<12>>,
    pub sbx: Reg<U<12>>,
    pub sby: Reg<U<12>>,
}
// end{state}

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
    /// a turn, and when the list is drawn wait for ever.
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
                    // pixel of a box, and of a triangle the pixels at
                    // which no edge function is negative.
                    let n0 = !self.e0.get().bit(31);
                    let n1 = !self.e1.get().bit(31);
                    let n2 = !self.e2.get().bit(31);
                    self.hit
                        .set((self.kind.get() != Kind::Tri) | (n0 & n1 & n2));
                    let open = self.issued.get() - self.answered.get();
                    self.inflight.set(open);
                    with!(self <= {
                        dgo ? answered: self.answered.get() + 1,
                    });
                    // Both a write's response and a read's beat give
                    // an identifier back, and one goes out a cycle.
                    if (dgo | got).to_bool() {
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
                    until(DefaultClock::rising, || issue.ready().to_bool()).await;
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
                    with!(self <= {
                        left: rdata.head().data.slice::<0, 8>(),
                        insn: U::<8>::from(0u8),
                    });
                    DefaultClock::rising().await;
                    if self.left.get() != 0 {
                        for _ in 0..self.left.get().raw() as usize {
                            // An edge for the instruction's index to
                            // read back, and for the sequence to
                            // begin a turn with.
                            DefaultClock::rising().await;
                            self.word.set(U::<3>::from(0u8));
                            // The instruction's six words, one read
                            // at a time, each latched as it lands.
                            for _ in 0..6 {
                                until(DefaultClock::rising, || {
                                    issue.ready().to_bool()
                                })
                                .await;
                                issue.send(Issue {
                                    read: Bit::One,
                                    addr: U::<A>::from(DL as u32)
                                        + (self.insn.get().resize::<A>()
                                            << SHIFT)
                                        + (self.word.get().resize::<A>() << WORD),
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
                                let rh = rdata.head();
                                self.word.set(self.word.get() + 1);
                                let word0 = rh.data.slice::<0, 2>();
                                // The setup the walk asks for: per
                                // edge, the two steps and the value at
                                // the box's first pixel. A vertex is
                                // two's complement and is widened by
                                // its sign; the box's first pixel is a
                                // screen coordinate and is widened by
                                // zero. The third vertex is the word
                                // that is landing.
                                let ax = self.sax.get().sext::<32>();
                                let ay = self.say.get().sext::<32>();
                                let bx = self.sbx.get().sext::<32>();
                                let by = self.sby.get().sext::<32>();
                                let cx = rh.data.slice::<0, 12>().sext::<32>();
                                let cy = rh.data.slice::<16, 12>().sext::<32>();
                                // The box. A clear says only its
                                // colour, so its box is the screen,
                                // which the rasteriser knows from its
                                // own type; a rectangle and a triangle
                                // carry theirs.
                                let clearing = self.skind.get() == Kind::Clear;
                                let zero16 = U::<16>::from(0u8);
                                let last_x =
                                    U::<16>::from(((1usize << LOGW) - 1) as u32);
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
                                let sx = wx.resize::<32>();
                                let sy = wy.resize::<32>();
                                let zero = U::<32>::from(0u8);
                                let t0x = zero - (by - ay);
                                let t0y = bx - ax;
                                let s0 = (bx - ax).mul::<32>(sy - ay)
                                    - (by - ay).mul::<32>(sx - ax);
                                let t1x = zero - (cy - by);
                                let t1y = cx - bx;
                                let s1 = (cx - bx).mul::<32>(sy - by)
                                    - (cy - by).mul::<32>(sx - bx);
                                let t2x = zero - (ay - cy);
                                let t2y = ax - cx;
                                let s2 = (ax - cx).mul::<32>(sy - cy)
                                    - (ay - cy).mul::<32>(sx - cx);
                                if self.word.get() == 0 {
                                    with!(self <= {
                                        skind: mux(
                                            word0 == 0,
                                            Kind::Clear,
                                            mux(word0 == 1, Kind::Rect, Kind::Tri),
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
                                        sax: rh.data.slice::<0, 12>(),
                                        say: rh.data.slice::<16, 12>(),
                                    });
                                }
                                if self.word.get() == 4 {
                                    with!(self <= {
                                        sbx: rh.data.slice::<0, 12>(),
                                        sby: rh.data.slice::<16, 12>(),
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
                                        e0: s0, r0: s0, d0x: t0x, d0y: t0y,
                                        e1: s1, r1: s1, d1x: t1x, d1y: t1y,
                                        e2: s2, r2: s2, d2x: t2x, d2y: t2y,
                                    });
                                }
                            }
                            // An edge, for the box to read back.
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
                                    let addr = (((py << LOGW) + px) << WORD)
                                        .resize::<A>()
                                        + U::<A>::from(BASE as u32);
                                    if self.hit.get().to_bool() {
                                        issue.send(Issue {
                                            read: Bit::Zero,
                                            addr,
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
                                            data: self.colour.get().resize::<32>(),
                                            strb: U::<4>::from(15u8),
                                            last: Bit::One,
                                        });
                                        self.issued.set(self.issued.get() + 1);
                                    }
                                    // The column advances and each edge
                                    // takes its column step.
                                    with!(self <= {
                                        x: px + 1,
                                        e0: self.e0.get() + self.d0x.get(),
                                        e1: self.e1.get() + self.d1x.get(),
                                        e2: self.e2.get() + self.d2x.get(),
                                    });
                                }
                                // The next row: the column goes back to
                                // the first and each edge is reloaded
                                // from its row value plus its row step.
                                let q0 = self.r0.get() + self.d0y.get();
                                let q1 = self.r1.get() + self.d1y.get();
                                let q2 = self.r2.get() + self.d2y.get();
                                with!(self <= {
                                    x: self.xa.get(),
                                    y: self.y.get() + 1,
                                    e0: q0, r0: q0,
                                    e1: q1, r1: q1,
                                    e2: q2, r2: q2,
                                });
                            }
                            self.insn.set(self.insn.get() + 1);
                        }
                        // The list is drawn; nothing follows.
                        self.finished.set(Bit::One);
                        until(DefaultClock::rising, || {
                            !self.finished.get().to_bool()
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
