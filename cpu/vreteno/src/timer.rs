// SPDX-License-Identifier: Apache-2.0
//! The timer, an AXI peripheral: the count and the compare, each in
//! two halves, four words at `TIMER_BASE`. It writes the lanes a
//! write's strobe covers into the word addressed, answers a read with
//! the word in the cycle it takes the burst, and raises its interrupt
//! while the count has reached the compare, through a register, so
//! that the line is a register's output and the core sees in a cycle
//! what the timer decided the cycle before. The count runs from the
//! reset, one a cycle. The router sends it only the bursts in its
//! range, so it checks no address.
use txhdl::comp::{mux, Clock, DefaultClock, In, Out, Reg, Rx, Tx, Unit};
use txhdl::types::{Bit, U};
use txhdl::{lower, select, with, Trace};
use txhdl_parts::bus::axi::{Answer, PerReq, Resp, R, W};

#[derive(Trace, Default)]
pub struct Timer<const I: usize> {
    pub mtime: Reg<U<64>>,
    pub mtimecmp: Reg<U<64>>,
    pub pending: Reg<Bit>,
    /// A write taken and waiting for its beat: which of the four
    /// words it names, and which identifier answers it.
    pub pend: Reg<U<1>>,
    pub psel: Reg<U<2>>,
    pub pid: Reg<U<I>>,
}

#[lower]
impl<const I: usize> Unit for Timer<I> {
    async fn run(
        &mut self,
        (rst, req, wd): (In<Bit>, Rx<PerReq<32, I>>, Rx<W<32, 4>>),
        (ans, rb, tirq): (Tx<Answer<I>>, Tx<R<32, I>>, Out<Bit>),
    ) {
        loop {
            DefaultClock::rising().await;
            let rst = rst.get();
            // The two words are sliced below, so they are read once.
            let (mtime, mtimecmp) = (self.mtime.get(), self.mtimecmp.get());
            let q = req.head();
            let qoff = req.peek().is_some();
            let held = self.pend.get() == 1;
            let take_read = qoff & q.read & rb.ready() & !held;
            let take_write = qoff & !q.read & !held;
            let _ = req.recv_if(take_read | take_write);
            let wh = wd.head();
            let wgo = held & wd.peek().is_some() & ans.ready();
            let _ = wd.recv_if(wgo);
            // The word a read names, and the word a held write names.
            let sel = q.addr.slice::<2, 2>();
            let word = select!(sel.raw() => {
                0 => mtime.slice::<0, 32>(),
                1 => mtime.slice::<32, 32>(),
                2 => mtimecmp.slice::<0, 32>(),
                _ => mtimecmp.slice::<32, 32>(),
            });
            let wsel = self.psel.get();
            let old = select!(wsel.raw() => {
                0 => mtime.slice::<0, 32>(),
                1 => mtime.slice::<32, 32>(),
                2 => mtimecmp.slice::<0, 32>(),
                _ => mtimecmp.slice::<32, 32>(),
            });
            // A write puts the lanes its strobe covers into the word.
            let wdata = wh.data;
            let strb = wh.strb;
            let merged =
                mux(strb.bit(3), wdata.slice::<24, 8>(), old.slice::<24, 8>())
                    .concat::<_, 16>(mux(
                        strb.bit(2),
                        wdata.slice::<16, 8>(),
                        old.slice::<16, 8>(),
                    ))
                    .concat::<_, 24>(mux(
                        strb.bit(1),
                        wdata.slice::<8, 8>(),
                        old.slice::<8, 8>(),
                    ))
                    .concat::<_, 32>(mux(
                        strb.bit(0),
                        wdata.slice::<0, 8>(),
                        old.slice::<0, 8>(),
                    ));
            self.mtime.set(mux(rst, U::<64>::from(0u32), mtime + 1));
            with!(self <= {
                take_write ? {
                    pend: U::<1>::from(1u8),
                    psel: sel,
                    pid: q.id,
                },
                wgo ? pend: U::<1>::from(0u8),
                wgo & (wsel == 0) ?
                    mtime: mtime.slice::<32, 32>().concat::<_, 64>(merged),
                wgo & (wsel == 1) ?
                    mtime: merged.concat::<_, 64>(mtime.slice::<0, 32>()),
                wgo & (wsel == 2) ? mtimecmp: mtimecmp
                    .slice::<32, 32>()
                    .concat::<_, 64>(merged),
                wgo & (wsel == 3) ? mtimecmp: merged
                    .concat::<_, 64>(mtimecmp.slice::<0, 32>()),
            });
            if take_read.to_bool() {
                rb.send(R {
                    id: q.id,
                    data: word,
                    resp: Resp::Okay,
                    last: Bit::One,
                });
            }
            if wgo.to_bool() {
                ans.send(Answer {
                    id: self.pid.get(),
                    resp: Resp::Okay,
                });
            }
            self.pending.set(mtime >= mtimecmp);
            tirq.set(self.pending);
        }
    }
}
