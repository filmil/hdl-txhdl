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
//! the whole of the line before it. The frame's base address is a
//! register taken at the vertical sync, so a host that draws into one
//! buffer and shows another flips them with one write. A column the
//! beam reads before its word has arrived sets `starved`, a sticky bit a
//! host reads and clears, so a run on the board can show that no line
//! ever starved rather than argue it.
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
//! 800 columns, and a visible line is 640 words, forty bursts of
//! sixteen. A line has the whole of the previous one to arrive in,
//! and a refresh lands about four times in it, so the margin is the
//! line time against forty bursts and four refreshes, not a burst's
//! latency against a pixel.
use txhdl::comp::{mux, Clock, DefaultClock, In, Mem, Out, Reg, Rx, Tx, Unit};
use txhdl::types::{Bit, U};
use txhdl::{lower, with, Trace};

// begin{pair}
/// Two lines of pixels on the clock that shows them, and the requests
/// that fill them.
///
/// `LEN` is the words in a visible line, `AW` the width of a column,
/// with `LEN` at most `1 << AW`. `ROWS` is the visible rows and
/// `TOTAL` the rows of a frame, blanking included. `STRIDE` is the
/// bytes from one line's start in memory to the next's.
#[derive(Trace)]
pub struct LinePair<
    const LEN: usize,
    const AW: usize,
    const ROWS: usize,
    const TOTAL: usize,
    const STRIDE: usize,
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
        C: Clock,
    > Default for LinePair<LEN, AW, ROWS, TOTAL, STRIDE, C>
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
        C: Clock,
    > Unit for LinePair<LEN, AW, ROWS, TOTAL, STRIDE, C>
{
    async fn run(
        &mut self,
        (inp, col, vis, line, row, frame, base, clear): (
            Rx<U<32>, C>,
            In<U<AW>, C>,
            In<Bit, C>,
            In<Bit, C>,
            In<U<12>, C>,
            In<Bit, C>,
            In<U<32>, C>,
            In<Bit, C>,
        ),
        (pix, req, starved): (Out<U<32>, C>, Tx<U<32>, C>, Out<Bit, C>),
    ) {
        loop {
            C::rising().await;
            // The outputs from the registers alone, before any input
            // is read, as every unit that another reads in the same
            // step has them.
            pix.set(self.shown.get());
            starved.set(self.under.get());
            // A word is taken whenever one is offered: the pair is
            // what the fetch is backpressured by, and a line longer
            // than `LEN` loses its tail rather than stalling.
            let take = Bit::from(inp.peek().is_some());
            let word = inp.recv_if(true).unwrap_or_default();
            let w = self.wsel.get();
            let at = self.at.get();
            let fits = at < U::<16>::from(LEN as u32);
            let to_a = take & fits & !w;
            let to_b = take & fits & w;
            let slot = at.resize::<AW>();
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
            let px = mux(rd, self.a.read(c), self.b.read(c));
            let got = mux(l, mux(take, at + 1, at), self.rgot.get());
            // A column shown before its word arrived.
            let starve =
                self.armed.get() & vis.get() & (c.resize::<16>() >= got);
            let r = row.get();
            let last = r == U::<12>::from((TOTAL - 1) as u32);
            let ask_first = l & last;
            let ask_next = l & !last & (r + 1 < U::<12>::from(ROWS as u32));
            let asked = (ask_first | ask_next) & req.ready();
            let addr = mux(ask_first, self.fbase.get(), self.next.get());
            let stride = U::<32>::from(STRIDE as u32);
            with!(self <= {
                shown: px,
                to_a ? a.at(slot): word,
                to_b ? b.at(slot): word,
                take ? at: at + 1,
                // A word taken at the edge a line starts on belongs to
                // the line just filled, so it counts there.
                l ? {
                    wsel: !w,
                    at: U::<16>::from(0u8),
                    rgot: got,
                },
                frame.get() ? fbase: base.get(),
                asked & ask_first ? {
                    next: self.fbase.get() + stride,
                    armed: Bit::One,
                },
                asked & ask_next ? next: self.next.get() + stride,
                clear.get() ? under: Bit::Zero,
                starve ? under: Bit::One,
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
