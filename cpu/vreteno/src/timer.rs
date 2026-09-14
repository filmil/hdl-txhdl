// SPDX-License-Identifier: Apache-2.0
//! The timer, a device on the bus: the count and the compare, each in
//! two halves, four words at `TIMER_BASE`. It takes every request
//! offered, writes the lanes a write covers into the word addressed,
//! answers a read with the word in the cycle after, and raises its
//! interrupt while the count has reached the compare, through a
//! register, so that the line is a register's output and the core
//! sees in a cycle what the timer decided the cycle before. The count
//! runs from the reset, one a cycle.
use crate::bus::{REQ_ADDR, REQ_WDATA};
use crate::isa::TIMER_BASE;
use txhdl::comp::{mux, Clock, DefaultClock, In, Out, Reg, Rx, Tx, Unit};
use txhdl::types::{Bit, U};
use txhdl::{lower, select, when, Trace};

#[derive(Trace, Default)]
pub struct Timer {
    pub mtime: Reg<U<64>>,
    pub mtimecmp: Reg<U<64>>,
    pub pending: Reg<Bit>,
}

#[lower]
impl Unit<(In<Bit>, Rx<U<69>>), (Tx<U<32>>, Out<Bit>)> for Timer {
    async fn run(
        &mut self,
        (rst, req): (In<Bit>, Rx<U<69>>),
        (resp, tirq): (Tx<U<32>>, Out<Bit>),
    ) {
        loop {
            DefaultClock::rising().await;
            let rst = rst.get();
            // The two words are sliced below, so they are read once.
            let (mtime, mtimecmp) = (self.mtime.get(), self.mtimecmp.get());
            // Every request is taken; the ones for this device are the
            // ones whose address falls in its sixteen bytes.
            let offered = req.peek().is_some();
            let r = req.recv().unwrap_or_default();
            let addr = r.slice::<REQ_ADDR, 32>();
            let wdata = r.slice::<REQ_WDATA, 32>();
            let we = r.bit(0); // REQ_WE, which the lowering wants literal
            let hit = addr.slice::<4, 28>() == TIMER_BASE >> 4;
            let sel = addr.slice::<2, 2>();
            let word = select!(sel.raw() => {
                0 => mtime.slice::<0, 32>(),
                1 => mtime.slice::<32, 32>(),
                2 => mtimecmp.slice::<0, 32>(),
                _ => mtimecmp.slice::<32, 32>(),
            });
            // A write puts the lanes it covers into the word; the lane
            // enables are bits 1 to 4 of the request.
            let merged =
                mux(r.bit(4), wdata.slice::<24, 8>(), word.slice::<24, 8>())
                    .concat::<8, 16>(mux(
                        r.bit(3),
                        wdata.slice::<16, 8>(),
                        word.slice::<16, 8>(),
                    ))
                    .concat::<8, 24>(mux(
                        r.bit(2),
                        wdata.slice::<8, 8>(),
                        word.slice::<8, 8>(),
                    ))
                    .concat::<8, 32>(mux(
                        r.bit(1),
                        wdata.slice::<0, 8>(),
                        word.slice::<0, 8>(),
                    ));
            let write = offered & we & hit;
            let read = offered & !we;
            self.mtime.set(mux(rst, U::<64>::from(0u32), mtime + 1));
            when!(write & (sel == 0) => {
                self.mtime <= mtime.slice::<32, 32>().concat::<32, 64>(merged)
            });
            when!(write & (sel == 1) => {
                self.mtime <= merged.concat::<32, 64>(mtime.slice::<0, 32>())
            });
            when!(write & (sel == 2) => {
                self.mtimecmp <=
                    mtimecmp.slice::<32, 32>().concat::<32, 64>(merged)
            });
            when!(write & (sel == 3) => {
                self.mtimecmp <=
                    merged.concat::<32, 64>(mtimecmp.slice::<0, 32>())
            });
            // A read is answered the cycle after it is taken, with the
            // word, or zero for an address that is not this device's.
            when!(read => { resp.send(mux(hit, word, U::<32>::from(0u32))) });
            self.pending.set(mtime >= mtimecmp);
            tirq.set(self.pending);
        }
    }
}
