// SPDX-License-Identifier: Apache-2.0
//! Reading memory in bursts, into a channel.
//!
//! A peripheral that wants a lot of memory does not want the core to
//! fetch it. The video scanout reads a line of a framebuffer per line
//! of the raster, and a MAC reads a frame per frame; done a word at a
//! time over AXI-Lite, each is a register write per word and the core
//! is a slow copier. Done here, the peripheral is a host on the bus
//! and the core says only where to start.
//!
//! `LineFetch` is the half both of those want: given an address and a
//! count, it issues read bursts and puts the words that come back
//! into a channel, one burst in flight. What consumes that channel is
//! the peripheral's business. For the video it crosses to the pixel
//! clock through [`crate::cdc::ChanCdc`] and feeds the raster; for a
//! MAC it feeds the transmitter.
//!
//! One burst at a time, deliberately. A second in flight would need a
//! second identifier, a tracker to tell the two apart, and an answer
//! to what happens when they come back out of order, and none of that
//! buys anything until the memory is faster than the consumer. The
//! consumer here is a raster taking a pixel per visible cycle.
use txhdl::comp::{mux, Clock, DefaultClock, In, Mem, Out, Reg, Rx, Tx, Unit};
use txhdl::types::{Bit, U};
use txhdl::{lower, with, Trace};

use crate::bus::axi::{BurstKind, Done, Grant, Issue, R, W};

// begin{state}
/// A reader of memory, into a channel.
///
/// `A` is the address width, `I` the width of a burst identifier, and
/// `BEATS` how many beats a burst asks for, which is the most the bus
/// will carry in one go. `WC` is the width of the word count, which
/// says how long a run is.
#[derive(Trace, Default)]
pub struct LineFetch<
    const A: usize,
    const I: usize,
    const BEATS: usize,
    const WC: usize,
> {
    /// Where the next burst starts.
    pub addr: Reg<U<A>>,
    /// Words still wanted in this run, across all its bursts.
    pub want: Reg<U<WC>>,
    /// Beats still to come back in the burst in flight.
    pub left: Reg<U<9>>,
    /// Whether a burst is out.
    pub busy: Reg<Bit>,
    /// The identifier the burst in flight was granted.
    pub id: Reg<U<I>>,
    /// Whether an identifier is held and not yet given back.
    pub held: Reg<Bit>,
}
// end{state}

// begin{run}
#[lower]
impl<const A: usize, const I: usize, const BEATS: usize, const WC: usize> Unit
    for LineFetch<A, I, BEATS, WC>
{
    async fn run(
        &mut self,
        (grant, done, rdata, base, words, go): (
            Rx<Grant<I>>,
            Rx<Done<I>>,
            Rx<R<32, I>>,
            In<U<A>>,
            In<U<WC>>,
            In<Bit>,
        ),
        (issue, release, out, running): (
            Tx<Issue<A>>,
            Tx<Grant<I>>,
            Tx<U<32>>,
            Out<Bit>,
        ),
    ) {
        loop {
            DefaultClock::rising().await;
            let want = self.want.get();
            let busy = self.busy.get();
            let held = self.held.get();
            let idle = (want == 0) & !busy;
            running.set(!idle);

            // A run starts when `go` is high and nothing is in
            // flight. The count and the base are read once, here, so
            // that a register written under the engine does not move
            // the line out from under it.
            let start = go.get() & idle;

            // The identifier comes back with the burst rather than
            // before it: the host takes an `Issue`, picks a free
            // identifier and says which on `grant`. So a burst is
            // sent first and the identifier is caught as it returns,
            // which is the opposite of asking for one and waiting.
            let take_id = Bit::from(grant.peek().is_some());
            let gid = grant.head().id;
            let _ = grant.recv_if(true);

            // A burst goes out when none is in flight, words are
            // still wanted and the address channel has room. It asks
            // for `BEATS` beats or the rest of the run, whichever is
            // fewer.
            let rest_small = want < U::<WC>::from(BEATS as u32);
            let beats =
                mux(rest_small, want.resize::<9>(), U::<9>::from(BEATS as u32));
            let send = !busy & !held & (want != 0) & issue.ready();

            // A beat is taken when one is offered and the channel out
            // has room, which is the backpressure: a consumer that
            // stops taking stops the bursts.
            //
            // A read burst is finished by its last beat, not by
            // `done`: the host puts a write's response there and
            // answers a read on the beat channel. So the identifier
            // goes back as the last beat is taken, and the last beat
            // is only taken when there is room to give it back.
            let at_last = self.left.get() == 1;
            let room = out.ready() & (!at_last | release.ready());
            let beat = Bit::from(rdata.peek().is_some()) & room;
            let word = rdata.recv_if(room).unwrap_or_default();
            let last = beat & at_last;
            let fin = last;
            // Nothing here writes, so nothing is owed on `done`. It is
            // drained so that a write answer arriving from elsewhere
            // cannot block the host.
            let _ = done.recv_if(true);

            with!(self <= {
                start ? { addr: base.get(), want: words.get() },
                take_id ? { id: gid, held: Bit::One },
                send ? {
                    busy: Bit::One,
                    left: beats,
                    addr: self.addr.get() + (beats.resize::<A>() << 2u32),
                    want: self.want.get() - beats.resize::<WC>(),
                },
                beat ? left: self.left.get() - 1,
                last ? busy: Bit::Zero,
                fin ? held: Bit::Zero,
            });

            if send.to_bool() {
                issue.send(Issue {
                    read: Bit::One,
                    addr: self.addr.get(),
                    len: (beats - 1).slice::<0, 8>(),
                    size: U::<3>::from(2u8),
                    burst: BurstKind::Incr,
                    lock: Bit::Zero,
                    cache: U::<4>::from(0u8),
                    prot: U::<3>::from(0u8),
                    qos: U::<4>::from(0u8),
                    region: U::<4>::from(0u8),
                });
            }
            if beat.to_bool() {
                out.send(word.data);
            }
            if fin.to_bool() {
                release.send(Grant { id: self.id.get() });
            }
        }
    }
}
// end{run}

// begin{linebuf}
/// The far end of a scanout: a line of pixels, on the clock that
/// shows them.
///
/// `LineFetch` reads a line from memory on the bus clock and
/// [`crate::cdc::ChanCdc`] carries the words to the pixel clock. This
/// is what they arrive in. A word taken lands at the next place in
/// the line; the raster asks for a column and gets what is there.
///
/// Writing and reading are independent on purpose. The raster reads a
/// column every visible cycle whatever the fetch is doing, and the
/// fetch fills ahead of the beam; nothing here enforces that it is
/// far enough ahead, because what "far enough" means belongs to the
/// video timing rather than to a buffer. A line that is not filled in
/// time shows the previous line's pixel at that column, which is a
/// visible artefact rather than a hang.
#[derive(Trace)]
pub struct LineBuf<const LEN: usize, const AW: usize, C: Clock> {
    /// The line. One word a pixel, which is what the framebuffer
    /// holds and what the crossing carries.
    pub px: Mem<U<32>, LEN, C>,
    /// Where the next word taken lands.
    pub at: Reg<U<AW>, C>,
    /// The pixel the column named at the last edge.
    ///
    /// Registered rather than read straight out of the memory,
    /// which is what a block RAM's output register is for: the
    /// pixel is then stable for the whole cycle it is shown, and the
    /// port does not carry a memory read that settles whenever the
    /// column moves. It costs the raster one cycle of lead, which it
    /// has.
    pub shown: Reg<U<32>, C>,
}
// end{linebuf}

/// Written out rather than derived, for the reason
/// [`crate::cdc::ChanCdc`] gives: a derived `Default` would ask the
/// clock to be `Default`, and a clock is a type with nothing in it.
impl<const LEN: usize, const AW: usize, C: Clock> Default
    for LineBuf<LEN, AW, C>
{
    fn default() -> Self {
        LineBuf {
            px: Mem::default(),
            at: Reg::default(),
            shown: Reg::default(),
        }
    }
}

#[lower]
impl<const LEN: usize, const AW: usize, C: Clock>
    Unit<(Rx<U<32>, C>, In<U<AW>, C>, In<Bit, C>), Out<U<32>, C>>
    for LineBuf<LEN, AW, C>
{
    async fn run(
        &mut self,
        (inp, col, sol): (Rx<U<32>, C>, In<U<AW>, C>, In<Bit, C>),
        pix: Out<U<32>, C>,
    ) {
        loop {
            C::rising().await;
            // A word is taken whenever one is offered: the buffer is
            // the consumer the fetch is backpressured by, so never
            // refusing is what keeps the line filling.
            let take = Bit::from(inp.peek().is_some());
            let word = inp.recv_if(true).unwrap_or_default();
            let start = sol.get();
            // The column the raster is asking for, read as the edge
            // left it, so the pixel is stable for the whole cycle it
            // is shown.
            pix.set(self.shown.get());
            with!(self <= {
                shown: self.px.read(col.get()),
                take ? { px.at(self.at.get()): word, at: self.at.get() + 1 },
                // The start of a line puts the write back to the
                // beginning, so a fetch that delivered too few words
                // does not walk the buffer out of step for ever.
                start ? at: U::<AW>::from(0u8),
            });
        }
    }
}

// begin{store}
/// Writing a run of memory from a channel, in bursts.
///
/// The mirror of [`LineFetch`], and the half an Ethernet receiver
/// wants: words arrive on a channel and are written to memory as
/// bursts. Where the fetcher is told where to read from and pushes
/// what it finds, this is told where to write and pulls what it is
/// given.
///
/// Two differences from the fetcher, and both are the bus rather than
/// a choice made here.
///
/// A write burst is finished by its response on `done`, not by its
/// last beat. The host answers a read on the beat channel and a write
/// on `done`, so the identifier goes back when the response arrives,
/// which may be well after the last beat went out.
///
/// AXI4 puts no identifier on the beats, so a burst's beats belong to
/// the oldest address phase outstanding. A second burst issued before
/// the first one's beats are out would take them. That is the other
/// reason for one burst at a time here, beyond the one the fetcher
/// gives: it is not an optimisation being declined, it is a rule.
#[derive(Trace, Default)]
pub struct LineStore<
    const A: usize,
    const I: usize,
    const BEATS: usize,
    const WC: usize,
> {
    /// Where the next burst starts.
    pub addr: Reg<U<A>>,
    /// Words still wanted in this run, across all its bursts.
    pub want: Reg<U<WC>>,
    /// Beats of the burst in flight still to go out.
    pub left: Reg<U<9>>,
    /// Whether a burst's beats are still going out.
    pub busy: Reg<Bit>,
    /// The identifier the burst in flight was granted.
    pub id: Reg<U<I>>,
    /// Whether an identifier is held and not yet given back.
    pub held: Reg<Bit>,
    /// How many bytes of the run's last word are real, zero meaning
    /// all four.
    ///
    /// A frame is a count of bytes and not of words. 1517 bytes is
    /// 380 words of which the last holds one real byte, and writing
    /// the other three would put whatever the channel happened to
    /// carry into memory past the end of the frame. The strobe on the
    /// last beat is what stops that, and this is what it is computed
    /// from.
    pub tail: Reg<U<2>>,
}
// end{store}

#[lower]
impl<const A: usize, const I: usize, const BEATS: usize, const WC: usize>
    Unit<
        (
            Rx<Grant<I>>,
            Rx<Done<I>>,
            Rx<U<32>>,
            In<U<A>>,
            In<U<WC>>,
            In<Bit>,
        ),
        (Tx<Issue<A>>, Tx<W<32, 4>>, Tx<Grant<I>>, Out<Bit>),
    > for LineStore<A, I, BEATS, WC>
{
    async fn run(
        &mut self,
        (grant, done, inp, base, bytes, go): (
            Rx<Grant<I>>,
            Rx<Done<I>>,
            Rx<U<32>>,
            In<U<A>>,
            In<U<WC>>,
            In<Bit>,
        ),
        (issue, wbeat, release, running): (
            Tx<Issue<A>>,
            Tx<W<32, 4>>,
            Tx<Grant<I>>,
            Out<Bit>,
        ),
    ) {
        loop {
            DefaultClock::rising().await;
            let want = self.want.get();
            let busy = self.busy.get();
            let held = self.held.get();
            let idle = (want == 0) & !busy & !held;
            running.set(!idle);

            let start = go.get() & idle;
            let take_id = Bit::from(grant.peek().is_some());
            let gid = grant.head().id;
            let _ = grant.recv_if(true);

            // The address phase. Nothing else is outstanding, because
            // the beats of a burst must all be out before the next
            // address phase goes.
            let rest_small = want < U::<WC>::from(BEATS as u32);
            let beats =
                mux(rest_small, want.resize::<9>(), U::<9>::from(BEATS as u32));
            let send = !busy & !held & (want != 0) & issue.ready();

            // A beat goes out when a word is offered and the beat
            // channel has room. A producer that stops offering stops
            // the burst, which is the backpressure in this direction.
            let at_last = self.left.get() == 1;
            let beat = busy & Bit::from(inp.peek().is_some()) & wbeat.ready();
            let word = inp.recv_if(busy & wbeat.ready()).unwrap_or_default();
            let last = beat & at_last;
            // The last beat OF THE RUN, not of the burst: `want` is
            // taken down when a burst is issued, so nothing is left
            // to ask for once it reaches zero.
            let tail = self.tail.get();
            let run_last = last & (want == 0) & (tail != 0);
            // Which bytes of that word are real: one, two or three,
            // and every byte otherwise.
            let part = mux(
                tail == 1,
                U::<4>::from(0x1u8),
                mux(tail == 2, U::<4>::from(0x3u8), U::<4>::from(0x7u8)),
            );
            let strb = mux(run_last, part, U::<4>::from(0xfu8));

            // The response, and the identifier back with it.
            let fin = Bit::from(done.peek().is_some()) & release.ready();
            let _ = done.recv_if(release.ready());

            with!(self <= {
                start ? {
                    addr: base.get(),
                    // Words, rounded up: a partial last word is still
                    // a beat, it is just not a whole one.
                    want: (bytes.get() + 3) >> 2u32,
                    tail: bytes.get().slice::<0, 2>(),
                },
                take_id ? { id: gid, held: Bit::One },
                send ? {
                    busy: Bit::One,
                    left: beats,
                    addr: self.addr.get() + (beats.resize::<A>() << 2u32),
                    want: self.want.get() - beats.resize::<WC>(),
                },
                beat ? left: self.left.get() - 1,
                last ? busy: Bit::Zero,
                fin ? held: Bit::Zero,
            });

            if send.to_bool() {
                issue.send(Issue {
                    read: Bit::Zero,
                    addr: self.addr.get(),
                    len: (beats - 1).slice::<0, 8>(),
                    size: U::<3>::from(2u8),
                    burst: BurstKind::Incr,
                    lock: Bit::Zero,
                    cache: U::<4>::from(0u8),
                    prot: U::<3>::from(0u8),
                    qos: U::<4>::from(0u8),
                    region: U::<4>::from(0u8),
                });
            }
            if beat.to_bool() {
                wbeat.send(W {
                    data: word,
                    strb,
                    last,
                });
            }
            if fin.to_bool() {
                release.send(Grant { id: self.id.get() });
            }
        }
    }
}
