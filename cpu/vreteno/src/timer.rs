// SPDX-License-Identifier: Apache-2.0
//! The core's interrupt controller, an AXI peripheral: the count and
//! the compare, each in two halves, and the software interrupt's one
//! bit, at the offsets every RISC-V platform puts them at, which is
//! what a stock port of an operating system looks for. It writes the
//! lanes a
//! write's strobe covers into the word addressed, answers a read with
//! the word in the cycle it takes the burst, and raises its interrupt
//! while the count has reached the compare, through a register, so
//! that the line is a register's output and the core sees in a cycle
//! what the timer decided the cycle before. The count runs from the
//! reset, one a cycle.
//!
//! | Offset | Name | What it is |
//! |---|---|---|
//! | `0x0000` | `msip` | bit 0 raises the software interrupt |
//! | `0x4000` | `mtimecmp` | the compare, low half |
//! | `0x4004` | | its high half |
//! | `0xbff8` | `mtime` | the count, low half |
//! | `0xbffc` | | its high half |
//!
//! The window is 64 KiB, which those offsets need, and the router
//! sends it only the bursts in it, so the decode is on the offset and
//! not on the whole address.
use txhdl::comp::{mux, Clock, DefaultClock, In, Out, Reg, Rx, Tx, Unit};
use txhdl::types::{Bit, U};
use txhdl::{lower, select, with, Trace};
use txhdl_parts::bus::axi::{Answer, PerReq, Resp, R, W};

/// Which of the five words an offset names, counting from zero:
/// `msip`, the compare's two halves, the count's two halves. An offset
/// the controller does not use reads as `msip` does, since a window of
/// 64 KiB holds far more words than five and none of the rest mean
/// anything.
#[lower]
fn word_of(addr: U<32>) -> U<3> {
    let off = addr.slice::<0, 16>();
    select!(off.raw() => {
        0x4000 => U::<3>::from(1u8),
        0x4004 => U::<3>::from(2u8),
        0xbff8 => U::<3>::from(3u8),
        0xbffc => U::<3>::from(4u8),
        _ => U::<3>::from(0u8),
    })
}

#[derive(Trace, Default)]
pub struct Timer<const I: usize> {
    pub mtime: Reg<U<64>>,
    pub mtimecmp: Reg<U<64>>,
    /// The software interrupt: bit 0 of the word at `msip`.
    pub msip: Reg<Bit>,
    pub pending: Reg<Bit>,
    /// A write taken and waiting for its beat: which of the four
    /// words it names, and which identifier answers it.
    pub pend: Reg<U<1>>,
    pub psel: Reg<U<3>>,
    pub pid: Reg<U<I>>,
}

#[lower]
impl<const I: usize> Unit for Timer<I> {
    async fn run(
        &mut self,
        (rst, req, wd): (In<Bit>, Rx<PerReq<32, I>>, Rx<W<32, 4>>),
        (ans, rb, tirq, sirq): (
            Tx<Answer<I>>,
            Tx<R<32, I>>,
            Out<Bit>,
            Out<Bit>,
        ),
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
            // Which of the five words an address names, as a number
            // from zero: the offsets are far apart, so the decode is
            // on the whole offset rather than on two bits of it, and
            // anything else in the window reads as zero and takes a
            // write nobody sees.
            let sel = word_of(q.addr);
            let word = select!(sel.raw() => {
                0 => self.msip.get().zext::<32>(),
                1 => mtimecmp.slice::<0, 32>(),
                2 => mtimecmp.slice::<32, 32>(),
                3 => mtime.slice::<0, 32>(),
                _ => mtime.slice::<32, 32>(),
            });
            let wsel = self.psel.get();
            let old = select!(wsel.raw() => {
                0 => self.msip.get().zext::<32>(),
                1 => mtimecmp.slice::<0, 32>(),
                2 => mtimecmp.slice::<32, 32>(),
                3 => mtime.slice::<0, 32>(),
                _ => mtime.slice::<32, 32>(),
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
                wgo & (wsel == 0) ? msip: Bit::from(merged.bit(0)),
                wgo & (wsel == 1) ? mtimecmp: mtimecmp
                    .slice::<32, 32>()
                    .concat::<_, 64>(merged),
                wgo & (wsel == 2) ? mtimecmp: merged
                    .concat::<_, 64>(mtimecmp.slice::<0, 32>()),
                wgo & (wsel == 3) ?
                    mtime: mtime.slice::<32, 32>().concat::<_, 64>(merged),
                wgo & (wsel == 4) ?
                    mtime: merged.concat::<_, 64>(mtime.slice::<0, 32>()),
                // The compare goes to all ones with the count to zero:
                // a compare left by the previous program would fire from
                // nowhere once the count reached it, and a compare of
                // zero would fire at once (issue 419).
                rst ? mtimecmp: U::<64>::from(u64::MAX),
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
            // The software line is the register itself: a program
            // raises it and clears it, and nothing else touches it.
            sirq.set(self.msip.get());
        }
    }
}
