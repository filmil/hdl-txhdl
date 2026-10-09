// SPDX-License-Identifier: Apache-2.0
//! A scanout from memory: the video's lines read from DDR3 a line
//! ahead of the beam (issue 151).
//!
//! The pieces are in [`crate::dma`] and [`crate::cdc`]: `LineFetch`
//! reads a run of memory in bursts into a channel on the bus clock,
//! and `ChanCdc` carries the words to the pixel clock. This adds the
//! two ends that make them a scanout.
//!
//! [`LinePair`] is on the pixel clock, beside the raster. It holds two
//! lines: the beam reads one while the other fills, and they change
//! places at the start of each line. At that start it asks for the
//! line after the one about to be shown, so a line is fetched during
//! the whole of the line before it. It asks for nothing while the
//! scanout is not shown, and once it is, it asks for a frame's first
//! line, from the base, before any other: out of a reset the next
//! line's address is zero, and the line there is the boot memory's,
//! whose one-beat answer to a burst hung the fetch for good on the
//! board (issue 1178). So the picture starts with the first frame
//! whose last row begins after the bit that shows it.
//!
//! The frame's base address is a register taken at the vertical sync,
//! so a host that draws into one buffer and shows another flips them
//! with one write. A column the beam reads before its word has arrived
//! sets `starved`, a sticky bit a host reads and clears, so a run on the
//! board can show that no line ever starved rather than argue it, and is
//! shown as [`LATE`], magenta, rather than as whatever word the line
//! before left there (issue 1209). A word belongs to the line it was
//! asked for, which the lines owed say (issue 1233): a line that starts
//! before it has come whole takes the rest of its words into the half
//! the beam reads, so its columns fill in as they come, and a line
//! whose time has passed has the rest of its words dropped, rather than
//! either landing in the half filling, where the next row would show
//! them as its own. A line
//! asked for that gets no word for two line times sets `stuck`, sticky
//! too, with the line's address: a fetch that hangs says so, where on
//! the board it once showed only as a black screen with `starved`
//! clear (issue 1197). A line not wholly inside the frame's memory,
//! the window the pair is given, is not asked for at all: it sets
//! `stuck` with its address at once and shows `LATE`, and the frame
//! goes on (issue 1382). A base gone wrong once sent a line's burst
//! into the serial port's page, and the core's prints stopped behind
//! it.
//!
//! [`ScanFetch`] is on the bus clock. It takes the line requests the
//! pixel side sends across and starts `LineFetch` on each, one at a
//! time.
//!
//! What a line must survive is the memory's worst latency, which on
//! the board is the MIG in `//ddr3`: an MT41K256M16 at 2.5 ns a clock,
//! `ddr3/mig/ddr3_mig.prj`, whose refresh takes tRFC = 260 ns once
//! every tREFI = 7.8 us, and whose bank miss is tRP + tRCD = 27.5 ns
//! before the CAS latency of six clocks, 15 ns. A read that meets a
//! refresh and then a miss waits about 300 ns plus the controller's
//! own pipeline. The board's mode is 640 by 480, 31.8 us a line of
//! 800 columns, and a visible line is 640 words. A line has the whole
//! of the previous one to arrive in, and a refresh lands about four
//! times in it, so the margin is the line time against the bursts, the
//! other hosts' turns between them and four refreshes, not a burst's
//! latency against a pixel. The flagship fetches a line in ten bursts
//! of 64 beats rather than forty of sixteen, since a burst pays the
//! path's latency and its turn at the arbiter once (issue 1209): in the
//! board's simulation, under the core copying in the DDR3 and the
//! Ethernet port sending, a line took 4682 cycles of its 3175 in
//! bursts of sixteen, and takes 1206.
use crate::bus::axi::Resp;
use crate::bus::axi_lite::{LiteAr, LiteAw, LiteB, LitePort, LiteR, LiteW};
use crate::bus::lite_split::LiteSplit;
use crate::hdmi::{Hdmi, Raster, VideoOut};
use txhdl::comp::{
    chan, join2, mux, signal, Clock, DefaultClock, In, Mem, Out, Reg, Rx, Tx,
    Unit,
};
use txhdl::types::{Bit, U};
use txhdl::{lower, regmap, with, Trace};
use txhdl::{Transaction as TransactionDerive, Value as ValueDerive};

/// What a column shows when its word has not arrived: magenta, which no
/// picture here uses, rather than the stale word the line before left
/// in the buffer, which looked like data out of place (issue 1209).
pub const LATE: u32 = 0x00ff_00ff;

// begin{pair}
/// Two lines of pixels on the clock that shows them, and the requests
/// that fill them.
///
/// `LEN` is the words in a visible line, `AW` the width of a column,
/// with `LEN` at most `1 << AW`. `ROWS` is the visible rows and
/// `TOTAL` the rows of a frame, blanking included. `STRIDE` is the
/// bytes from one line's start in memory to the next's. `LO` and `HI`
/// are the frame's memory, the bytes from `LO` up to but not including
/// `HI`: a line not wholly inside them is not asked for (issue 1382).
#[derive(Trace)]
pub struct LinePair<
    const LEN: usize,
    const AW: usize,
    const ROWS: usize,
    const TOTAL: usize,
    const STRIDE: usize,
    const LO: usize,
    const HI: usize,
    C: Clock,
> {
    /// One line.
    pub a: Mem<U<32>, LEN, C>,
    /// The other.
    pub b: Mem<U<32>, LEN, C>,
    /// Which line is filling: `b` when set, `a` when clear. The beam
    /// reads the other.
    pub wsel: Reg<Bit, C>,
    /// The words that have arrived in the line filling.
    pub at: Reg<U<16>, C>,
    /// The words that arrived in the line the beam reads.
    pub rgot: Reg<U<16>, C>,
    /// The pixel the column named at the last edge.
    pub shown: Reg<U<32>, C>,
    /// A column was shown before its word arrived. Sticky.
    pub under: Reg<Bit, C>,
    /// The first line of a frame has been asked for, so a short line
    /// is a fault from here on and not the start of the run.
    pub armed: Reg<Bit, C>,
    /// The frame's base address, taken at the vertical sync.
    pub fbase: Reg<U<32>, C>,
    /// Where the next line to ask for starts.
    pub next: Reg<U<32>, C>,
    /// The address of the oldest line asked for and not yet come
    /// whole (issue 1197).
    pub owed0: Reg<U<32>, C>,
    /// The next oldest's.
    pub owed1: Reg<U<32>, C>,
    /// The third's.
    pub owed2: Reg<U<32>, C>,
    /// The fourth's: no more are asked for while four are owed.
    pub owed3: Reg<U<32>, C>,
    /// How many lines are owed.
    pub owing: Reg<U<3>, C>,
    /// How many words of the oldest line owed have come.
    pub came: Reg<U<16>, C>,
    /// A word has come since the last line started.
    pub heard: Reg<Bit, C>,
    /// How many line starts in a row found lines owed and no word
    /// come.
    pub quiet: Reg<U<2>, C>,
    /// A line asked for got no word for two line times. Sticky.
    pub hung: Reg<Bit, C>,
    /// That line's address.
    pub hung_at: Reg<U<32>, C>,
    /// The line being shown had not come whole when it started, so its
    /// words, as they come, go into the half the beam reads (issue
    /// 1233).
    pub late: Reg<Bit, C>,
    /// Lines owed whose time has passed: their words are dropped rather
    /// than written where another line's go (issue 1233).
    pub behind: Reg<U<3>, C>,
    /// Lines asked for whose rows have not begun (issue 1523).
    pub ahead: Reg<U<3>, C>,
    /// Lines are asked for two rows ahead rather than one, as `two`
    /// said at the last vertical sync.
    pub two_ahead: Reg<Bit, C>,
    /// A count of the pixel clock's edges, which a line's time is
    /// measured in.
    pub tick: Reg<U<16>, C>,
    /// The row each line owed is for, oldest first: two rows ahead, a
    /// line's words wait in the crossing until the row before its own,
    /// whose half is the one it is shown from.
    pub g0: Reg<U<12>, C>,
    /// The next oldest's.
    pub g1: Reg<U<12>, C>,
    /// The third's.
    pub g2: Reg<U<12>, C>,
    /// The fourth's.
    pub g3: Reg<U<12>, C>,
    /// The tick each line owed was asked for at, oldest first.
    pub t0: Reg<U<16>, C>,
    /// The next oldest's.
    pub t1: Reg<U<16>, C>,
    /// The third's.
    pub t2: Reg<U<16>, C>,
    /// The fourth's.
    pub t3: Reg<U<16>, C>,
    /// The longest any line took from being asked for to its last
    /// word, in pixels. Kept until cleared.
    pub longest: Reg<U<16>, C>,
    /// How many lines had not come whole when their rows began. Kept
    /// until cleared, and stops at its largest.
    pub late_lines: Reg<U<16>, C>,
}
// end{pair}

/// Written out for the reason [`crate::dma::LineBuf`] gives: a derived
/// `Default` would ask the clock to be `Default`.
impl<
        const LEN: usize,
        const AW: usize,
        const ROWS: usize,
        const TOTAL: usize,
        const STRIDE: usize,
        const LO: usize,
        const HI: usize,
        C: Clock,
    > Default for LinePair<LEN, AW, ROWS, TOTAL, STRIDE, LO, HI, C>
{
    fn default() -> Self {
        LinePair {
            a: Mem::default(),
            b: Mem::default(),
            wsel: Reg::default(),
            at: Reg::default(),
            rgot: Reg::default(),
            shown: Reg::default(),
            under: Reg::default(),
            armed: Reg::default(),
            fbase: Reg::default(),
            next: Reg::default(),
            owed0: Reg::default(),
            owed1: Reg::default(),
            owed2: Reg::default(),
            owed3: Reg::default(),
            owing: Reg::default(),
            came: Reg::default(),
            heard: Reg::default(),
            quiet: Reg::default(),
            hung: Reg::default(),
            hung_at: Reg::default(),
            late: Reg::default(),
            behind: Reg::default(),
            ahead: Reg::default(),
            two_ahead: Reg::default(),
            tick: Reg::default(),
            g0: Reg::default(),
            g1: Reg::default(),
            g2: Reg::default(),
            g3: Reg::default(),
            t0: Reg::default(),
            t1: Reg::default(),
            t2: Reg::default(),
            t3: Reg::default(),
            longest: Reg::default(),
            late_lines: Reg::default(),
        }
    }
}

// begin{pair_run}
#[lower]
impl<
        const LEN: usize,
        const AW: usize,
        const ROWS: usize,
        const TOTAL: usize,
        const STRIDE: usize,
        const LO: usize,
        const HI: usize,
        C: Clock,
    > Unit for LinePair<LEN, AW, ROWS, TOTAL, STRIDE, LO, HI, C>
{
    async fn run(
        &mut self,
        (inp, col, vis, line, row, frame, base, clear, show, two): (
            Rx<U<32>, C>,
            In<U<AW>, C>,
            In<Bit, C>,
            In<Bit, C>,
            In<U<12>, C>,
            In<Bit, C>,
            In<U<32>, C>,
            In<Bit, C>,
            In<Bit, C>,
            In<Bit, C>,
        ),
        (pix, req, starved, stuck, stuck_at, worst, lates): (
            Out<U<32>, C>,
            Tx<U<32>, C>,
            Out<Bit, C>,
            Out<Bit, C>,
            Out<U<32>, C>,
            Out<U<16>, C>,
            Out<U<16>, C>,
        ),
    ) {
        loop {
            C::rising().await;
            // The outputs from the registers alone, before any input
            // is read, as every unit that another reads in the same
            // step has them.
            pix.set(self.shown.get());
            starved.set(self.under.get());
            stuck.set(self.hung.get());
            stuck_at.set(self.hung_at.get());
            worst.set(self.longest.get());
            lates.set(self.late_lines.get());
            let w = self.wsel.get();
            let at = self.at.get();
            // A word is taken whenever one is offered, one row ahead:
            // the pair is what the fetch is backpressured by, and a line
            // longer than `LEN` loses its tail rather than stalling. Two
            // rows ahead, a line's words wait in the crossing until the
            // row before its own, when the half filling is the one its
            // row is shown from (issue 1523). The halves change places at
            // every row's start, the blanking's too.
            let r_now = row.get();
            let after = r_now + U::<12>::from(1u8);
            let next_row = mux(
                after == U::<12>::from(TOTAL as u32),
                U::<12>::from(0u8),
                after,
            );
            // A word taken at the edge a row starts on belongs to the
            // half just filled, the starting row's, as below.
            let fills = mux(line.get(), r_now, next_row);
            let waits = self.two_ahead.get()
                & !self.late.get()
                & (self.behind.get() == U::<3>::from(0u8))
                & (self.owing.get() != U::<3>::from(0u8))
                & (self.g0.get() != fills);
            let take = Bit::from(inp.peek().is_some()) & !waits;
            let word = inp.recv_if(!waits.to_bool()).unwrap_or_default();
            // Where a word goes (issue 1233). A word of a line whose
            // time has passed is dropped; one of the line being shown,
            // which started before it came whole, goes into the half the
            // beam reads, at its own column; any other goes into the
            // half filling, as before.
            let came = self.came.get();
            let drop = take & (self.behind.get() != U::<3>::from(0u8));
            let to_shown = take & !drop & self.late.get();
            let normal = take & !drop & !self.late.get();
            let fits = at < U::<16>::from(LEN as u32);
            let fits_late = came < U::<16>::from(LEN as u32);
            // The beam reads `a` while `b` fills, and `b` while `a` does.
            let into_a = (normal & fits & !w) | (to_shown & fits_late & w);
            let into_b = (normal & fits & w) | (to_shown & fits_late & !w);
            let slot = mux(to_shown, came.resize::<AW>(), at.resize::<AW>());
            // At a line's start the two change places, and the line
            // after the one about to be shown is asked for. The last
            // row of the frame asks for the frame's first.
            let l = line.get();
            // The beam reads the line not filling: at a line's start,
            // the one that has just filled, which is what it reads for
            // the rest of the line, so its first column is not the
            // last line's.
            let c = col.get();
            let rd = w ^ l;
            // A column past the line is the blanking's, whose pixel is
            // not shown: the line is read at its first word there rather
            // than past its end, which a raster wider than the line names
            // (issue 1194).
            let inside = c.resize::<16>() < U::<16>::from(LEN as u32);
            let rc = mux(inside, c, U::<AW>::from(0u8));
            let px = mux(rd, self.a.read(rc), self.b.read(rc));
            let got = mux(l, mux(normal, at + 1, at), self.rgot.get());
            // A column shown before its word arrived.
            let starve =
                self.armed.get() & vis.get() & (c.resize::<16>() >= got);
            let r = r_now;
            // The row a line start asks for: the next, or two rows on
            // when the pair asks two ahead (issue 1523), which a host
            // turns on and off for the frame after the vertical sync.
            // Two rows ahead a line has a whole row longer to come, and
            // the crossing holds it until its half is free.
            let two_now = mux(frame.get(), two.get(), self.two_ahead.get());
            let d = mux(two_now, U::<12>::from(2u8), U::<12>::from(1u8));
            let total = U::<12>::from(TOTAL as u32);
            let on_row = r + d;
            let target = mux(on_row >= total, on_row - total, on_row);
            // Nothing is asked for until the pair is shown, and then the
            // frame's first line first, from the base a host gave it:
            // out of a reset the next address is zero, and memory there
            // is the boot memory, which answers a line's burst with one
            // beat and hangs the fetch for good (issue 1178).
            let on = show.get();
            let first_row = target == U::<12>::from(0u8);
            let ask_first = l & first_row & on;
            let ask_next = l
                & !first_row
                & self.armed.get()
                & (target < U::<12>::from(ROWS as u32));
            // At most four lines are owed at once, which a fetch that
            // keeps up never comes near: it is one line ahead.
            let owing = self.owing.get();
            let room = owing < U::<3>::from(4u8);
            let due = (ask_first | ask_next) & req.ready() & room;
            // The first line after the scanout is shown is asked from the
            // base as the host gave it, not from the one taken at the last
            // vertical sync, which a reset leaves at zero: a show written
            // after a sync and before the last row began asked from zero,
            // and the fetch walked into the boot memory and the serial
            // port (issue 1317). Once armed, frames take the base at the
            // sync, so a host still flips with one write.
            // Two rows ahead, the first line may be asked at the very
            // edge the base is taken at, so the base is read through.
            let first_at = mux(
                self.armed.get() & !frame.get(),
                self.fbase.get(),
                base.get(),
            );
            let addr = mux(ask_first, first_at, self.next.get());
            let stride = U::<32>::from(STRIDE as u32);
            // A line outside the frame's memory is not asked for (issue
            // 1382): a base gone wrong once sent a line's burst of 64
            // beats into the serial port's page, which held the path
            // the core prints through. The frame goes on as if it had
            // been, so the rows after it are asked where they belong;
            // the line is never filled, so it shows `LATE`, and `stuck`
            // says where it was.
            let in_mem = Bit::from(addr >= U::<32>::from(LO as u32))
                & Bit::from(addr <= U::<32>::from((HI - 4 * LEN) as u32));
            let asked = due & in_mem;
            let refused = due & !in_mem;
            // The lines owed (issue 1197). A word counts against the
            // oldest, and the last of its words takes it off; a line
            // asked for goes after those still owed. A line that gets no
            // word for two line starts in a row did not come: `stuck`
            // is set, sticky, with its address, and the scanout goes on
            // asking, so the bit says why the screen is black.
            let counted = take & (owing != U::<3>::from(0u8));
            let whole = counted & (came + 1 == U::<16>::from(LEN as u32));
            let one = U::<3>::from(1u8);
            let none = U::<3>::from(0u8);
            let pos = owing - mux(whole, one, none);
            let (o0, o1, o2, o3) = (
                self.owed0.get(),
                self.owed1.get(),
                self.owed2.get(),
                self.owed3.get(),
            );
            let n0 = mux(asked & (pos == none), addr, mux(whole, o1, o0));
            let n1 = mux(asked & (pos == one), addr, mux(whole, o2, o1));
            let n2 = mux(
                asked & (pos == U::<3>::from(2u8)),
                addr,
                mux(whole, o3, o2),
            );
            let n3 = mux(
                asked & (pos == U::<3>::from(3u8)),
                addr,
                mux(whole, U::<32>::from(0u8), o3),
            );
            let owing_next =
                owing + mux(asked, one, none) - mux(whole, one, none);
            // When each line owed was asked for, kept beside its address,
            // and the longest a line took to come whole (issue 1523).
            let tick = self.tick.get();
            let (q0, q1, q2, q3) =
                (self.t0.get(), self.t1.get(), self.t2.get(), self.t3.get());
            let (h0, h1, h2, h3) =
                (self.g0.get(), self.g1.get(), self.g2.get(), self.g3.get());
            let e0 = mux(asked & (pos == none), target, mux(whole, h1, h0));
            let e1 = mux(asked & (pos == one), target, mux(whole, h2, h1));
            let e2 = mux(
                asked & (pos == U::<3>::from(2u8)),
                target,
                mux(whole, h3, h2),
            );
            let e3 = mux(
                asked & (pos == U::<3>::from(3u8)),
                target,
                mux(whole, U::<12>::from(0u8), h3),
            );
            let m0 = mux(asked & (pos == none), tick, mux(whole, q1, q0));
            let m1 = mux(asked & (pos == one), tick, mux(whole, q2, q1));
            let m2 = mux(
                asked & (pos == U::<3>::from(2u8)),
                tick,
                mux(whole, q3, q2),
            );
            let m3 = mux(
                asked & (pos == U::<3>::from(3u8)),
                tick,
                mux(whole, U::<16>::from(0u8), q3),
            );
            let took = tick - q0;
            let longer = whole & (took > self.longest.get());
            // A visible row begins, and the line asked for it is due.
            let starts =
                l & self.armed.get() & (r < U::<12>::from(ROWS as u32));
            let ahead = self.ahead.get();
            let still = mux(starts & (ahead != none), ahead - one, ahead);
            // At a line's start, the lines still owed once this step's
            // word is counted, less those asked for rows still to come:
            // none is on time; one is the line about to be shown, late;
            // more are that line and lines whose time has passed, whose
            // words are dropped.
            let k = owing - mux(whole, one, none);
            let late_now = k > still;
            let behind_now = mux(k > still + one, k - still - one, none);
            let n_late = self.late_lines.get();
            let counted_late =
                starts & late_now & (n_late != U::<16>::from(0xffffu32));
            let behind = self.behind.get();
            let heard = self.heard.get() | take;
            let quiet = self.quiet.get();
            let silent = l & (owing != none) & !heard;
            let lost = silent & (quiet != U::<2>::from(0u8));
            with!(self <= {
                // A column whose word has not arrived shows LATE, not the
                // word left there by the line before (issue 1209).
                shown: mux(starve, U::<32>::from(LATE), px),
                into_a ? a.at(slot): word,
                into_b ? b.at(slot): word,
                normal ? at: at + 1,
                // A late line's word counts as come in the half shown.
                to_shown & !l ? rgot: came + 1,
                // A word taken at the edge a line starts on belongs to
                // the line just filled, so it counts there.
                l ? {
                    wsel: !w,
                    at: U::<16>::from(0u8),
                    rgot: got,
                },
                frame.get() ? fbase: base.get(),
                due & ask_first ? {
                    next: first_at + stride,
                    armed: Bit::One,
                },
                due & ask_next ? next: self.next.get() + stride,
                !on ? armed: Bit::Zero,
                clear.get() ? under: Bit::Zero,
                starve ? under: Bit::One,
                owed0: n0,
                owed1: n1,
                owed2: n2,
                owed3: n3,
                owing: owing_next,
                came: mux(whole, U::<16>::from(0u8), mux(counted, came + 1, came)),
                heard: mux(l, take, heard),
                l ? quiet: mux(silent, quiet + 1, U::<2>::from(0u8)),
                clear.get() ? { hung: Bit::Zero, quiet: U::<2>::from(0u8) },
                lost & !self.hung.get() ? {
                    hung: Bit::One,
                    hung_at: o0,
                },
                refused & !self.hung.get() ? {
                    hung: Bit::One,
                    hung_at: addr,
                },
                // A line come whole while late: a passed one, one fewer
                // to drop; else the line shown, which is no longer late.
                !l & whole & (behind != none) ? behind: behind - one,
                !l & whole & (behind == none) ? late: Bit::Zero,
                l ? {
                    late: late_now,
                    behind: behind_now,
                },
                tick: tick + 1,
                g0: e0,
                g1: e1,
                g2: e2,
                g3: e3,
                t0: m0,
                t1: m1,
                t2: m2,
                t3: m3,
                ahead: still + mux(due, one, none),
                !on ? ahead: none,
                frame.get() ? two_ahead: two.get(),
                longer ? longest: took,
                counted_late ? late_lines: n_late + 1,
                clear.get() ? {
                    longest: U::<16>::from(0u8),
                    late_lines: U::<16>::from(0u8),
                },
            });
            if asked.to_bool() {
                req.send(addr);
            }
        }
    }
}
// end{pair_run}

// begin{fetch}
/// The bus side: a line request taken from the crossing, and
/// `LineFetch` started on it.
///
/// `A` is the address width, `WC` the width of the word count and
/// `LEN` the words in a line. One request at a time: the next is taken
/// only once the fetch has started on this one and finished, which
/// `running` says.
#[derive(Trace, Default)]
pub struct ScanFetch<const A: usize, const WC: usize, const LEN: usize> {
    /// Where the line asked for starts.
    pub base: Reg<U<A>>,
    /// High for the one cycle that starts the fetch.
    pub go: Reg<Bit>,
    /// A request has been handed on and the fetch has not yet said it
    /// is running.
    pub pend: Reg<Bit>,
}
// end{fetch}

// begin{fetch_run}
#[lower]
impl<const A: usize, const WC: usize, const LEN: usize> Unit
    for ScanFetch<A, WC, LEN>
{
    async fn run(
        &mut self,
        (lines, running): (Rx<U<32>>, In<Bit>),
        (at, count, start): (Out<U<A>>, Out<U<WC>>, Out<Bit>),
    ) {
        loop {
            DefaultClock::rising().await;
            // To `LineFetch`'s `base`, `words` and `go`.
            at.set(self.base.get());
            count.set(U::<WC>::from(LEN as u32));
            start.set(self.go.get());
            let run = running.get();
            let free = !run & !self.pend.get() & !self.go.get();
            let take = free & Bit::from(lines.peek().is_some());
            let addr = lines.head();
            let _ = lines.recv_if(take);
            with!(self <= {
                take ? {
                    base: addr.resize::<A>(),
                    go: Bit::One,
                    pend: Bit::One,
                },
                self.go.get() ? go: Bit::Zero,
                run ? pend: Bit::Zero,
            });
        }
    }
}
// end{fetch_run}

// begin{regs}
// The scanout's AXI-Lite words, in the upper half of the video slot.
regmap! { scan (scan_read, scan_we), 3: [
    (0, base, rw, "the byte the next frame starts at in memory"),
    (1, ctrl, rw, "what the screen shows", [
        (scan, 0, 1, rw, 0, "the scanout when set, the framebuffer when clear"),
        (ahead, 1, 1, rw, 0, "lines asked two rows ahead, from the next frame"),
    ]),
    (2, status, ro, "how the scanout has kept up", [
        (under, 0, 1, ro, 0, "a column was shown before its word arrived"),
        (stuck, 1, 1, ro, 0, "a line asked for got no word for two line times"),
    ]),
    (3, clear, wo, "a write clears the status, the longest line and the count"),
    (4, stuck_at, ro, "the address of the line that did not come"),
    (5, worst, ro, "the longest a line took to come whole, in pixels"),
    (6, lates, ro, "how many lines had not come whole when their rows began"),
] }

/// What the pair says about how it has kept up, for [`ScanCtl`]: the
/// underflow bit, and the stuck bit with the address of the line that
/// did not come (issue 1197).
#[derive(TransactionDerive, ValueDerive, Clone, Copy, Default, Debug)]
pub struct ScanState {
    /// A column was shown before its word arrived.
    pub under: Bit,
    /// A line asked for got no word for two line times.
    pub stuck: Bit,
    /// That line's address.
    pub at: U<32>,
    /// The longest a line took to come whole, in pixels (issue 1523).
    pub worst: U<16>,
    /// How many lines were late when their rows began.
    pub lates: U<16>,
}
// end{regs}

// begin{ctl}
/// The scanout's registers on AXI-Lite, `scan`: the frame's base, the
/// bit that picks the scanout over the framebuffer, and the underflow
/// and stuck bits, read and cleared, with the address of a line that
/// did not come.
///
/// Its three outputs are registers, for [`LinePair`] and the video
/// multiplexer to read. The underflow bit comes back from [`LinePair`]
/// over a channel, through [`ScanTap`], rather than on a wire: this
/// unit drives `clear`, which [`LinePair`] reads, so it runs before
/// [`LinePair`], and a wire back from it would be read a step stale.
/// A channel commits at the end of the step whichever ran first. The
/// bit read here is therefore up to three cycles behind the pair's,
/// in the run and the netlist alike, which a host reading it after a
/// frame does not see.
#[derive(Trace, Default)]
pub struct ScanCtl {
    /// Where the next frame starts.
    pub fbase: Reg<U<32>>,
    /// Show the scanout.
    pub show: Reg<Bit>,
    /// High for the one cycle after a write to `clear`.
    pub clr: Reg<Bit>,
    /// The underflow bit, as the channel last said it.
    pub seen: Reg<Bit>,
    /// The stuck bit, the same way (issue 1197).
    pub seen_stuck: Reg<Bit>,
    /// The line that did not come, the same way.
    pub seen_at: Reg<U<32>>,
    /// Ask for lines two rows ahead (issue 1523).
    pub ahead: Reg<Bit>,
    /// The longest line, as the channel last said it.
    pub seen_worst: Reg<U<16>>,
    /// The count of late lines, the same way.
    pub seen_lates: Reg<U<16>>,
}
// end{ctl}

// begin{ctl_run}
#[lower]
impl Unit for ScanCtl {
    async fn run(
        &mut self,
        bus: LitePort<32, 32, 4>,
        (under, base, mode, clear, two): (
            Rx<ScanState>,
            Out<U<32>>,
            Out<Bit>,
            Out<Bit>,
            Out<Bit>,
        ),
    ) {
        loop {
            DefaultClock::rising().await;
            base.set(self.fbase.get());
            mode.set(self.show.get());
            clear.set(self.clr.get());
            two.set(self.ahead.get());
            let heard = Bit::from(under.peek().is_some());
            let said = under.head();
            let _ = under.recv_if(heard);
            let arh = bus.ar.head();
            let awh = bus.aw.head();
            let rsel = arh.addr.slice::<2, 3>();
            let wsel = awh.addr.slice::<2, 3>();
            let rgo = bus.r.ready() & bus.ar.peek().is_some();
            let _ = bus.ar.recv_if(bus.r.ready());
            let wgo = bus.b.ready()
                & bus.aw.peek().is_some()
                & bus.w.peek().is_some();
            let _ = bus.aw.recv_if(wgo);
            let _ = bus.w.recv_if(wgo);
            let written = bus.w.head().data;
            let we = scan_we(wgo, wsel);
            let word = scan_read(
                rsel,
                self.fbase.get(),
                scan_ctrl_pack(self.show.get(), self.ahead.get()),
                scan_status_pack(self.seen.get(), self.seen_stuck.get()),
                U::<32>::from(0u8),
                self.seen_at.get(),
                self.seen_worst.get().resize::<32>(),
                self.seen_lates.get().resize::<32>(),
            );
            with!(self <= {
                we.bit(0) ? fbase: written,
                we.bit(1) ? {
                    show: scan_ctrl_scan(written),
                    ahead: scan_ctrl_ahead(written),
                },
                clr: we.bit(3),
                heard ? {
                    seen: said.under,
                    seen_stuck: said.stuck,
                    seen_at: said.at,
                    seen_worst: said.worst,
                    seen_lates: said.lates,
                },
            });
            if rgo.to_bool() {
                bus.r.send(LiteR {
                    data: word,
                    resp: Resp::Okay,
                });
            }
            if wgo.to_bool() {
                bus.b.send(LiteB { resp: Resp::Okay });
            }
        }
    }
}
// end{ctl_run}

// begin{tap}
/// [`LinePair`]'s underflow and stuck bits, and the line that did not
/// come, onto a channel for [`ScanCtl`]: wires read after [`LinePair`]
/// has driven them, sent on whenever the channel has room.
#[derive(Trace, Default)]
pub struct ScanTap {}

#[lower]
impl Unit for ScanTap {
    async fn run(
        &mut self,
        (starved, stuck, at, worst, lates): (
            In<Bit>,
            In<Bit>,
            In<U<32>>,
            In<U<16>>,
            In<U<16>>,
        ),
        tap: Tx<ScanState>,
    ) {
        loop {
            DefaultClock::rising().await;
            if tap.ready().to_bool() {
                tap.send(ScanState {
                    under: starved.get(),
                    stuck: stuck.get(),
                    at: at.get(),
                    worst: worst.get(),
                    lates: lates.get(),
                });
            }
        }
    }
}
// end{tap}

// begin{vmux}
/// What the screen shows: [`Hdmi`]'s picture with its own pixel, or
/// the same timing with the scanout's. Wires only, read after both
/// have driven them, and both pixels are a cycle behind the column
/// that named them, so they line up with the syncs [`Hdmi`] delays to
/// meet its own. A scanout pixel is `0x00RRGGBB`.
#[derive(Trace, Default)]
pub struct VidMux {}

#[lower]
impl Unit for VidMux {
    async fn run(
        &mut self,
        (mode, pix, rgb_in, hs_in, vs_in, de_in): (
            In<Bit>,
            In<U<32>>,
            In<U<24>>,
            In<Bit>,
            In<Bit>,
            In<Bit>,
        ),
        (rgb, hsync, vsync, de): (Out<U<24>>, Out<Bit>, Out<Bit>, Out<Bit>),
    ) {
        loop {
            DefaultClock::rising().await;
            let shown = mux(mode.get(), pix.get().resize::<24>(), rgb_in.get());
            rgb.set(shown);
            hsync.set(hs_in.get());
            vsync.set(vs_in.get());
            de.set(de_in.get());
        }
    }
}
// end{vmux}

/// The address bit that splits the video slot: [`Hdmi`]'s words below
/// it, [`ScanCtl`]'s from `1 << SCAN_BIT`. A program's map adds this to
/// the slot's base, and a test holds the two to each other.
pub const SCAN_BIT: usize = 7;

// begin{video}
/// The video peripheral with a scanout beside it, on the pixel clock:
/// what the board's third slot holds (issue 151).
///
/// The slot is split by address bit 7: [`Hdmi`]'s words below `0x80`,
/// as they always were, and [`ScanCtl`]'s from `0x80`. [`Raster`]
/// counts the beam again for [`LinePair`], which asks for lines on
/// `req` and takes their words on `words`, both of which leave this
/// unit for the bus clock. [`VidMux`] puts either picture on the pins.
///
/// `HV` to `VBP` and `SHIFT` are [`Hdmi`]'s. `AW` is the width of a
/// column, `TOTAL` the rows of a frame, which must be `VV + VFP + VSW +
/// VBP`, `STRIDE` the bytes from one line to the next in memory, and
/// `LO` to `HI` the frame's memory, outside which no line is asked for.
/// `TOTAL` is stated because a parameter cannot be a sum of others
/// here; the `Default` refuses one that disagrees.
///
/// The children run in an order that puts every wire's driver before
/// its reader: [`Raster`] and [`ScanCtl`], then [`LinePair`], then
/// [`ScanTap`] and [`Hdmi`], then [`VidMux`]. The one path the other
/// way, the underflow bit back to [`ScanCtl`], is a channel.
#[derive(Trace)]
pub struct ScanVideo<
    const HV: usize,
    const HFP: usize,
    const HSW: usize,
    const HBP: usize,
    const VV: usize,
    const VFP: usize,
    const VSW: usize,
    const VBP: usize,
    const SHIFT: usize,
    const AW: usize,
    const TOTAL: usize,
    const STRIDE: usize,
    const LO: usize,
    const HI: usize,
> {
    /// The slot, split by address bit 7.
    pub split: LiteSplit<32, 32, 4, SCAN_BIT>,
    /// The scanout's registers, from `0x80`.
    pub ctl: ScanCtl,
    /// The beam, counted again for the pair.
    pub raster: Raster<HV, HFP, HSW, HBP, VV, VFP, VSW, VBP, AW>,
    /// The two lines.
    pub pair: LinePair<HV, AW, VV, TOTAL, STRIDE, LO, HI, DefaultClock>,
    /// The underflow bit back to the registers.
    pub tap: ScanTap,
    /// The video peripheral, below `0x80`.
    pub hdmi: Hdmi<HV, HFP, HSW, HBP, VV, VFP, VSW, VBP, SHIFT>,
    /// Which picture reaches the pins.
    pub vmux: VidMux,
}
// end{video}

impl<
        const HV: usize,
        const HFP: usize,
        const HSW: usize,
        const HBP: usize,
        const VV: usize,
        const VFP: usize,
        const VSW: usize,
        const VBP: usize,
        const SHIFT: usize,
        const AW: usize,
        const TOTAL: usize,
        const STRIDE: usize,
        const LO: usize,
        const HI: usize,
    > Default
    for ScanVideo<
        HV,
        HFP,
        HSW,
        HBP,
        VV,
        VFP,
        VSW,
        VBP,
        SHIFT,
        AW,
        TOTAL,
        STRIDE,
        LO,
        HI,
    >
{
    fn default() -> Self {
        assert_eq!(TOTAL, VV + VFP + VSW + VBP, "TOTAL is the frame's rows");
        assert!(HV <= 1 << AW, "a visible line's columns fit in AW bits");
        ScanVideo {
            split: LiteSplit::default(),
            ctl: ScanCtl::default(),
            raster: Raster::default(),
            pair: LinePair::default(),
            tap: ScanTap::default(),
            hdmi: Hdmi::default(),
            vmux: VidMux::default(),
        }
    }
}

// begin{video_run}
#[lower]
impl<
        const HV: usize,
        const HFP: usize,
        const HSW: usize,
        const HBP: usize,
        const VV: usize,
        const VFP: usize,
        const VSW: usize,
        const VBP: usize,
        const SHIFT: usize,
        const AW: usize,
        const TOTAL: usize,
        const STRIDE: usize,
        const LO: usize,
        const HI: usize,
    > Unit
    for ScanVideo<
        HV,
        HFP,
        HSW,
        HBP,
        VV,
        VFP,
        VSW,
        VBP,
        SHIFT,
        AW,
        TOTAL,
        STRIDE,
        LO,
        HI,
    >
{
    async fn run(
        &mut self,
        bus: LitePort<32, 32, 4>,
        (words, req, rgb, hsync, vsync, de): (
            Rx<U<32>>,
            Tx<U<32>>,
            Out<U<24>>,
            Out<Bit>,
            Out<Bit>,
            Out<Bit>,
        ),
    ) {
        // The two halves of the slot, a channel each way per AXI-Lite
        // channel: the split drives each `_tx` of an address or a word
        // and reads each `_rx` of an answer.
        let (lo_aw, lo_aw_rx) = chan::<LiteAw<32>, DefaultClock>();
        let (lo_ar, lo_ar_rx) = chan::<LiteAr<32>, DefaultClock>();
        let (lo_w, lo_w_rx) = chan::<LiteW<32, 4>, DefaultClock>();
        let (lo_b_tx, lo_b) = chan::<LiteB, DefaultClock>();
        let (lo_r_tx, lo_r) = chan::<LiteR<32>, DefaultClock>();
        let (hi_aw, hi_aw_rx) = chan::<LiteAw<32>, DefaultClock>();
        let (hi_ar, hi_ar_rx) = chan::<LiteAr<32>, DefaultClock>();
        let (hi_w, hi_w_rx) = chan::<LiteW<32, 4>, DefaultClock>();
        let (hi_b_tx, hi_b) = chan::<LiteB, DefaultClock>();
        let (hi_r_tx, hi_r) = chan::<LiteR<32>, DefaultClock>();
        let (col_o, col) = signal::<U<AW>, DefaultClock>();
        let (vis_o, vis) = signal::<Bit, DefaultClock>();
        let (line_o, line) = signal::<Bit, DefaultClock>();
        let (row_o, row) = signal::<U<12>, DefaultClock>();
        let (frame_o, frame) = signal::<Bit, DefaultClock>();
        let (base_o, base) = signal::<U<32>, DefaultClock>();
        let (mode_o, mode) = signal::<Bit, DefaultClock>();
        // The bit that shows the scanout is also what lets the pair ask
        // for lines at all (issue 1178).
        let show = mode.clone();
        let (clear_o, clear) = signal::<Bit, DefaultClock>();
        let (pix_o, pix) = signal::<U<32>, DefaultClock>();
        let (starved_o, starved) = signal::<Bit, DefaultClock>();
        let (stuck_o, stuck) = signal::<Bit, DefaultClock>();
        let (stuck_at_o, stuck_at) = signal::<U<32>, DefaultClock>();
        let (worst_o, worst) = signal::<U<16>, DefaultClock>();
        let (lates_o, lates) = signal::<U<16>, DefaultClock>();
        let (two_o, two) = signal::<Bit, DefaultClock>();
        let (tap_tx, tap_rx) = chan::<ScanState, DefaultClock>();
        let (hrgb_o, hrgb) = signal::<U<24>, DefaultClock>();
        let (hhs_o, hhs) = signal::<Bit, DefaultClock>();
        let (hvs_o, hvs) = signal::<Bit, DefaultClock>();
        let (hde_o, hde) = signal::<Bit, DefaultClock>();
        join2(
            join2(
                join2(
                    self.split.run(
                        bus,
                        (
                            lo_aw, lo_ar, lo_w, lo_b, lo_r, hi_aw, hi_ar, hi_w,
                            hi_b, hi_r,
                        ),
                    ),
                    self.raster.run((), (col_o, vis_o, line_o, row_o, frame_o)),
                ),
                join2(
                    self.ctl.run(
                        LitePort {
                            aw: hi_aw_rx,
                            ar: hi_ar_rx,
                            w: hi_w_rx,
                            b: hi_b_tx,
                            r: hi_r_tx,
                        },
                        (tap_rx, base_o, mode_o, clear_o, two_o),
                    ),
                    self.pair.run(
                        (
                            words, col, vis, line, row, frame, base, clear,
                            show, two,
                        ),
                        (
                            pix_o, req, starved_o, stuck_o, stuck_at_o,
                            worst_o, lates_o,
                        ),
                    ),
                ),
            ),
            join2(
                join2(
                    self.tap
                        .run((starved, stuck, stuck_at, worst, lates), tap_tx),
                    self.hdmi.run(
                        LitePort {
                            aw: lo_aw_rx,
                            ar: lo_ar_rx,
                            w: lo_w_rx,
                            b: lo_b_tx,
                            r: lo_r_tx,
                        },
                        VideoOut {
                            rgb: hrgb_o,
                            hsync: hhs_o,
                            vsync: hvs_o,
                            de: hde_o,
                        },
                    ),
                ),
                self.vmux.run(
                    (mode, pix, hrgb, hhs, hvs, hde),
                    (rgb, hsync, vsync, de),
                ),
            ),
        )
        .await;
    }
}
// end{video_run}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hdmi::{vga, Raster};
    use txhdl::comp::Running;

    /// `ctrl` reads back as written, both its bits, so a host may set
    /// one and keep the other (issue 1523): the HAL's `show` and
    /// `two_ahead` each read `ctrl` and write it back with their own bit
    /// changed.
    #[test]
    fn ctrl_reads_back_as_written() {
        use crate::bus::axi_lite::{LiteAr, LiteAw, LiteB, LiteR, LiteW};
        let mut ctl = ScanCtl::default();
        let (aw_tx, aw) = chan::<LiteAw<32>, DefaultClock>();
        let (ar_tx, ar) = chan::<LiteAr<32>, DefaultClock>();
        let (w_tx, w) = chan::<LiteW<32, 4>, DefaultClock>();
        let (b, b_rx) = chan::<LiteB, DefaultClock>();
        let (r, r_rx) = chan::<LiteR<32>, DefaultClock>();
        let (_tap_tx, tap_rx) = chan::<ScanState, DefaultClock>();
        let (base_o, _base) = signal::<U<32>, DefaultClock>();
        let (mode_o, mode) = signal::<Bit, DefaultClock>();
        let (clear_o, _clear) = signal::<Bit, DefaultClock>();
        let (two_o, two) = signal::<Bit, DefaultClock>();
        let mut sim = Running::new(ctl.run(
            LitePort { aw, ar, w, b, r },
            (tap_rx, base_o, mode_o, clear_o, two_o),
        ));
        let at = |word: u32| LiteAw::<32> {
            addr: U::<32>::from(4 * word),
            prot: U::<3>::from(0u8),
        };
        let write = |sim: &mut Running<_>, word: u32, v: u32| {
            aw_tx.send(at(word));
            w_tx.send(LiteW {
                data: U::<32>::from(v),
                strb: U::<4>::from(0xfu8),
            });
            for _ in 0..20 {
                sim.cycle();
                if b_rx.recv_if(true).is_some() {
                    return;
                }
            }
            panic!("no answer to a write");
        };
        let read = |sim: &mut Running<_>, word: u32| -> u32 {
            ar_tx.send(at(word));
            for _ in 0..20 {
                sim.cycle();
                if let Some(a) = r_rx.recv_if(true) {
                    return a.data.raw() as u32;
                }
            }
            panic!("no answer to a read");
        };
        for v in [1u32, 3, 2, 0, 1] {
            write(&mut sim, 1, v);
            for _ in 0..4 {
                sim.cycle();
            }
            assert_eq!(read(&mut sim, 1), v, "ctrl after writing {v}");
            assert_eq!(mode.get().to_bool(), v & 1 == 1, "show for {v}");
            assert_eq!(two.get().to_bool(), v & 2 == 2, "ahead for {v}");
        }
    }

    /// The flagship's frame, 480 rows of 525, its words given as fast as
    /// the pair takes them, one row ahead and two (issue 1523): from the
    /// second frame on, no line is late at its row and no column is
    /// shown before its word, so a late line on the board is a late
    /// word and not the frame's own shape.
    #[test]
    fn a_frame_with_its_words_on_time_has_no_late_line() {
        type R = Raster<
            { vga::HV },
            { vga::HFP },
            { vga::HSW },
            { vga::HBP },
            { vga::VV },
            { vga::VFP },
            { vga::VSW },
            { vga::VBP },
            10,
        >;
        type P = LinePair<
            { vga::HV },
            10,
            { vga::VV },
            525,
            4096,
            0x4000_0000,
            0x8000_0000,
            DefaultClock,
        >;
        for ahead in [false, true] {
            let mut raster = R::default();
            let mut pair = P::default();
            let (col_o, col) = signal::<U<10>, DefaultClock>();
            let (vis_o, vis) = signal::<Bit, DefaultClock>();
            let (line_o, line) = signal::<Bit, DefaultClock>();
            let (row_o, row) = signal::<U<12>, DefaultClock>();
            let (frame_o, frame) = signal::<Bit, DefaultClock>();
            let (base_o, base) = signal::<U<32>, DefaultClock>();
            let (clear_o, clear) = signal::<Bit, DefaultClock>();
            let (show_o, show) = signal::<Bit, DefaultClock>();
            let (pix_o, _pix) = signal::<U<32>, DefaultClock>();
            let (starved_o, starved) = signal::<Bit, DefaultClock>();
            let (stuck_o, stuck) = signal::<Bit, DefaultClock>();
            let (stuck_at_o, _stuck_at) = signal::<U<32>, DefaultClock>();
            let (worst_o, worst) = signal::<U<16>, DefaultClock>();
            let (lates_o, lates) = signal::<U<16>, DefaultClock>();
            let (two_o, two) = signal::<Bit, DefaultClock>();
            let (words_tx, words) = chan::<U<32>, DefaultClock>();
            let (req, req_rx) = chan::<U<32>, DefaultClock>();
            base_o.set(U::<32>::from(0x4100_0000u32));
            clear_o.set(Bit::Zero);
            show_o.set(Bit::One);
            two_o.set(Bit::from_bool(ahead));
            let mut sim = Running::new(join2(
                raster.run((), (col_o, vis_o, line_o, row_o, frame_o)),
                pair.run(
                    (words, col, vis, line, row, frame, base, clear, show, two),
                    (
                        pix_o, req, starved_o, stuck_o, stuck_at_o, worst_o,
                        lates_o,
                    ),
                ),
            ));
            let frame_len = 525 * 800;
            let mut owed = 0usize;
            for c in 0..3 * frame_len {
                // The bits are cleared once the first frame is over.
                clear_o.set(Bit::from_bool(c == frame_len + 1000));
                if req_rx.recv_if(true).is_some() {
                    owed += vga::HV;
                }
                if owed > 0 && words_tx.ready().to_bool() {
                    words_tx.send(U::<32>::from(owed as u32));
                    owed -= 1;
                }
                sim.cycle();
            }
            let n = lates.get().raw();
            assert_eq!(n, 0, "two ahead {ahead}: {n} lines late");
            assert!(!starved.get().to_bool(), "two ahead {ahead}: starved");
            assert!(!stuck.get().to_bool(), "two ahead {ahead}: stuck");
            let w = worst.get().raw();
            assert!(w < 2 * 800 + 640, "two ahead {ahead}: longest {w}");
        }
    }

    /// The flagship's pair and raster, a line of 640 words in 800
    /// columns: the beam names columns 640 to 799 in the blanking, past
    /// the line, and the pair must not read its line there (issue 1194).
    /// Two lines are run, shown from the start, with the words of each
    /// line given as they are asked for.
    #[test]
    fn the_blanking_columns_read_nothing_past_the_line() {
        type R = Raster<
            { vga::HV },
            { vga::HFP },
            { vga::HSW },
            { vga::HBP },
            { vga::VV },
            { vga::VFP },
            { vga::VSW },
            { vga::VBP },
            10,
        >;
        type P = LinePair<
            { vga::HV },
            10,
            { vga::VV },
            525,
            4096,
            0x4000_0000,
            0x8000_0000,
            DefaultClock,
        >;
        let mut raster = R::default();
        let mut pair = P::default();
        let (col_o, col) = signal::<U<10>, DefaultClock>();
        let (vis_o, vis) = signal::<Bit, DefaultClock>();
        let (line_o, line) = signal::<Bit, DefaultClock>();
        let (row_o, row) = signal::<U<12>, DefaultClock>();
        let (frame_o, frame) = signal::<Bit, DefaultClock>();
        let (base_o, base) = signal::<U<32>, DefaultClock>();
        let (clear_o, clear) = signal::<Bit, DefaultClock>();
        let (show_o, show) = signal::<Bit, DefaultClock>();
        let (pix_o, _pix) = signal::<U<32>, DefaultClock>();
        let (starved_o, _starved) = signal::<Bit, DefaultClock>();
        let (stuck_o, _stuck) = signal::<Bit, DefaultClock>();
        let (stuck_at_o, _stuck_at) = signal::<U<32>, DefaultClock>();
        let (worst_o, _worst) = signal::<U<16>, DefaultClock>();
        let (lates_o, _lates) = signal::<U<16>, DefaultClock>();
        let (_two_o, two) = signal::<Bit, DefaultClock>();
        let (words_tx, words) = chan::<U<32>, DefaultClock>();
        let (req, req_rx) = chan::<U<32>, DefaultClock>();
        base_o.set(U::<32>::from(0x4100_0000u32));
        clear_o.set(Bit::Zero);
        show_o.set(Bit::One);
        let mut sim = Running::new(join2(
            raster.run((), (col_o, vis_o, line_o, row_o, frame_o)),
            pair.run(
                (words, col, vis, line, row, frame, base, clear, show, two),
                (pix_o, req, starved_o, stuck_o, stuck_at_o, worst_o, lates_o),
            ),
        ));
        let mut owed = 0usize;
        for _ in 0..2 * 800 {
            if req_rx.recv_if(true).is_some() {
                owed += vga::HV;
            }
            if owed > 0 && words_tx.ready().to_bool() {
                words_tx.send(U::<32>::from(owed as u32));
                owed -= 1;
            }
            sim.cycle();
        }
    }
}
