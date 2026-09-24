// SPDX-License-Identifier: Apache-2.0
//! The framebuffer: `N` words of memory behind an AXI peripheral end,
//! one word per pixel, `N` a power of two.
//!
//! It answers single-beat bursts, which is all the rasteriser makes.
//! A read is answered from the memory in the cycle the request is
//! taken; a write is held until its beat arrives, because AXI4 puts
//! no identifier on the write data channel and the beat therefore
//! comes when it comes. The word address is the byte address shifted
//! by two and masked to the memory, so a stray address wraps instead
//! of ending the run.
use txhdl::comp::{Clock, DefaultClock, Mem, Reg, Unit};
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
            // A read is answered at once; a write is held until its
            // beat arrives, and only one is held at a time.
            let take_read = qoff & q.read & bus.r.ready() & !held;
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
            });
            if take_read.to_bool() {
                bus.r.send(R {
                    id: q.id,
                    data: self.px.read(at),
                    resp: Resp::Okay,
                    last: Bit::One,
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
}
