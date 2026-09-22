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
use txhdl::comp::{mux, Clock, DefaultClock, In, Out, Reg, Rx, Tx, Unit};
use txhdl::types::{Bit, U};
use txhdl::{lower, with, Trace};

use crate::bus::axi::{BurstKind, Done, Grant, Issue, R};

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
