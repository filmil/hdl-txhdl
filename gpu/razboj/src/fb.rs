// SPDX-License-Identifier: Apache-2.0
//! The framebuffer: `N` words of memory behind an AXI peripheral end,
//! one word per pixel, `N` a power of two.
//!
//! It answers single-beat writes, which is all the rasteriser makes,
//! and read bursts of any length, since the rasteriser fetches a
//! display list entry as one. A read's first beat is answered from the
//! memory in the cycle the request is taken and the rest follow a beat
//! a cycle; a write is held until its beat arrives, because AXI4 puts
//! no identifier on the write data channel and the beat therefore
//! comes when it comes. The word address is the byte address shifted
//! by two and masked to the memory, so a stray address wraps instead
//! of ending the run.
use txhdl::comp::{mux, Clock, DefaultClock, Mem, Reg, Unit};
use txhdl::types::{Bit, U};
use txhdl::{lower, with, Trace};
use txhdl_parts::bus::axi::{Answer, PerPort, Resp, R};

/// A pixel is one word, so the word a byte address names is the
/// address shifted by this.
const WORD: usize = 2;

// begin{state}
/// The framebuffer, `N` words of one pixel each.
#[derive(Trace, Default)]
pub struct Fb<const A: usize, const I: usize, const N: usize> {
    pub px: Mem<U<32>, N>,
    /// A write taken and waiting for its beat: where it goes and
    /// which identifier answers it.
    pub pend: Reg<U<1>>,
    pub paddr: Reg<U<16>>,
    pub pid: Reg<U<I>>,
    /// A read burst being answered after its first beat: the beats
    /// still to send, the word the next one reads, and the identifier
    /// every beat carries.
    pub rleft: Reg<U<8>>,
    pub raddr: Reg<U<16>>,
    pub rid: Reg<U<I>>,
    /// Read bursts taken and read beats sent, for a run to count what
    /// the rasteriser asked of the memory.
    pub rbursts: Reg<U<32>>,
    pub rbeats: Reg<U<32>>,
}
// end{state}

// begin{run}
#[lower]
impl<const A: usize, const I: usize, const N: usize> Unit for Fb<A, I, N> {
    async fn run(&mut self, bus: PerPort<A, 32, 4, I>, _o: ()) {
        loop {
            DefaultClock::rising().await;
            let q = bus.req.head();
            let qoff = bus.req.peek().is_some();
            let held = self.pend.get() == 1;
            // The word this request names, within the memory. The
            // shift comes before the widening, so an address wider
            // than the index keeps its high bits until they are
            // masked away.
            let at =
                (q.addr >> WORD).resize::<16>() & U::<16>::from((N - 1) as u32);
            // A read's first beat is answered at once and the rest a
            // beat a cycle, one burst at a time; a write is held until
            // its beat arrives, and only one is held at a time.
            let mask = U::<16>::from((N - 1) as u32);
            let reading = Bit::from(self.rleft.get() != 0);
            let more = reading & bus.r.ready();
            let take_read = qoff & q.read & bus.r.ready() & !held & !reading;
            let take_write = qoff & !q.read & !held;
            let _ = bus.req.recv_if(take_read | take_write);
            let wh = bus.w.head();
            let wgo = held & bus.w.peek().is_some() & bus.ans.ready();
            let _ = bus.w.recv_if(wgo);
            with!(self <= {
                take_write ? {
                    pend: U::<1>::from(1u8),
                    paddr: at,
                    pid: q.id,
                },
                wgo ? {
                    pend: U::<1>::from(0u8),
                    px.at(self.paddr.get()): wh.data,
                },
                take_read ? {
                    rleft: q.len,
                    raddr: (at + 1) & mask,
                    rid: q.id,
                    rbursts: self.rbursts.get() + 1,
                    rbeats: self.rbeats.get() + 1,
                },
                more ? {
                    rleft: self.rleft.get() - 1,
                    raddr: (self.raddr.get() + 1) & mask,
                    rbeats: self.rbeats.get() + 1,
                },
            });
            // One beat a cycle: a burst's first, or the next of the one
            // being answered.
            let rat = mux(more, self.raddr.get(), at);
            if (take_read | more).to_bool() {
                bus.r.send(R {
                    id: mux(more, self.rid.get(), q.id),
                    data: self.px.read(rat),
                    resp: Resp::Okay,
                    last: mux(
                        more,
                        Bit::from(self.rleft.get() == 1),
                        Bit::from(q.len == 0),
                    ),
                });
            }
            if wgo.to_bool() {
                bus.ans.send(Answer {
                    id: self.pid.get(),
                    resp: Resp::Okay,
                });
            }
        }
    }
}
// end{run}

impl<const A: usize, const I: usize, const N: usize> Fb<A, I, N> {
    /// A pixel, for the run and the tests to look at.
    pub fn pixel(&self, at: usize) -> u32 {
        self.px.read(at).raw() as u32
    }
    /// Read bursts taken and read beats sent so far.
    pub fn reads(&self) -> (u64, u64) {
        (
            self.rbursts.get().raw() as u64,
            self.rbeats.get().raw() as u64,
        )
    }
}
