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
use txhdl::comp::{mux, Clock, DefaultClock, In, Out, Reg, Unit};
use txhdl::types::{Bit, U};
use txhdl::{lower, select, with, Trace};
use txhdl_parts::bus::axi::{Answer, PerPort, Resp, R};

/// Which of the five words an offset names, counting from zero:
/// `msip`, the compare's two halves, the count's two halves; and 5 for
/// an offset the controller does not use, which reads zero and takes a
/// write nobody sees, as the reference model has it. It used to be
/// `msip`, so a stray store raised the software interrupt (issue 681).
#[lower]
fn word_of(addr: U<32>) -> U<3> {
    let off = addr.slice::<0, 16>();
    select!(off.raw() => {
        0x0000 => U::<3>::from(0u8),
        0x4000 => U::<3>::from(1u8),
        0x4004 => U::<3>::from(2u8),
        0xbff8 => U::<3>::from(3u8),
        0xbffc => U::<3>::from(4u8),
        _ => U::<3>::from(5u8),
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
        bus: PerPort<32, 32, 4, I>,
        (rst, tirq, sirq): (In<Bit>, Out<Bit>, Out<Bit>),
    ) {
        loop {
            DefaultClock::rising().await;
            let rst = rst.get();
            // The two words are sliced below, so they are read once.
            let (mtime, mtimecmp) = (self.mtime.get(), self.mtimecmp.get());
            let q = bus.req.head();
            let qoff = bus.req.peek().is_some();
            let held = self.pend.get() == 1;
            let take_read = qoff & q.read & bus.r.ready() & !held;
            let take_write = qoff & !q.read & !held;
            let _ = bus.req.recv_if(take_read | take_write);
            let wh = bus.w.head();
            let wgo = held & bus.w.peek().is_some() & bus.ans.ready();
            let _ = bus.w.recv_if(wgo);
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
                4 => mtime.slice::<32, 32>(),
                _ => U::<32>::from(0u8),
            });
            let wsel = self.psel.get();
            let old = select!(wsel.raw() => {
                0 => self.msip.get().zext::<32>(),
                1 => mtimecmp.slice::<0, 32>(),
                2 => mtimecmp.slice::<32, 32>(),
                3 => mtime.slice::<0, 32>(),
                4 => mtime.slice::<32, 32>(),
                _ => U::<32>::from(0u8),
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
                bus.r.send(R {
                    id: q.id,
                    data: word,
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
            self.pending.set(mtime >= mtimecmp);
            tirq.set(self.pending);
            // The software line is the register itself: a program
            // raises it and clears it, and nothing else touches it.
            sirq.set(self.msip.get());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Timer;
    use txhdl::comp::{chan, signal, DefaultClock, Running, Unit};
    use txhdl::types::{Bit, U};
    use txhdl_parts::bus::axi::{PerPort, PerReq, W};

    /// Write `data` at `off` in the timer's window, then read `off`:
    /// what the read answered, and `msip` after both.
    fn write_then_read(off: u32, data: u32) -> (u32, bool) {
        let mut t = Timer::<2>::default();
        let msip = t.msip;
        let (req_tx, req) = chan::<PerReq<32, 2>, DefaultClock>();
        let (w_tx, w) = chan();
        let (ans, ans_rx) = chan();
        let (r, r_rx) = chan();
        let (_rst_out, rst) = signal::<Bit, DefaultClock>();
        let (tirq, _tirq) = signal::<Bit, DefaultClock>();
        let (sirq, _sirq) = signal::<Bit, DefaultClock>();
        let port = PerPort { req, w, ans, r };
        let mut sim = Running::new(t.run(port, (rst, tirq, sirq)));
        let at = U::<32>::from(0x0200_0000 + off);
        let req_at = |read| PerReq {
            read,
            id: U::from(1u8),
            addr: at,
            size: U::from(2u8),
            ..PerReq::default()
        };
        req_tx.send(req_at(Bit::Zero));
        w_tx.send(W {
            data: U::from(data),
            strb: U::from(0xfu8),
            last: Bit::One,
        });
        for _ in 0..4 {
            sim.cycle();
            let _ = ans_rx.recv();
        }
        req_tx.send(req_at(Bit::One));
        let mut got = None;
        for _ in 0..4 {
            sim.cycle();
            if let Some(beat) = r_rx.recv() {
                got = Some(beat.data.raw() as u32);
            }
        }
        (got.expect("the read was answered"), msip.get().to_bool())
    }

    /// `msip` itself: written one, it reads one and is set. This is
    /// what shows the bench reaches the register at all.
    #[test]
    fn a_write_of_one_to_msip_raises_it() {
        assert_eq!(write_then_read(0x0000, 1), (1, true));
    }

    /// A word of the window that names nothing reads zero and takes a
    /// write nobody sees, as the reference model has it; it used to be
    /// taken as `msip`, so a stray store raised the software interrupt
    /// (issue 681).
    #[test]
    fn a_write_to_an_unused_offset_leaves_msip_alone() {
        assert_eq!(write_then_read(0x0008, 1), (0, false));
    }
}
