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
//! The map is `regmap!`'s `clint`, below: `msip` at `0x0000`, the
//! compare at `0x4000` and the count at `0xbff8`, each in two halves,
//! low first. It writes the read mux and the write enables, and the
//! offsets `isa` and the tests address by (issue 668).
//!
//! The window is 64 KiB, which those offsets need, and the router
//! sends it only the bursts in it, so the decode is on the offset and
//! not on the whole address.
use txhdl::comp::{mux, Clock, DefaultClock, In, Out, Reg, Unit};
use txhdl::regmap;
use txhdl::types::{Bit, U};
use txhdl::{lower, with, Trace};
use txhdl_parts::bus::axi::{Answer, PerPort, Resp, R};

// begin{map}
// The map: the five words at the offsets every RISC-V platform puts
// them at, as word indices, fourteen address bits above the byte bits
// selecting one of the window's 16384 words (issues 499 and 668).
regmap! { clint (clint_read, clint_we), 14: [
    (0x0000, msip, rw, "the software interrupt", [
        (msip, 0, 1, rw, 0, "one raises the software interrupt"),
    ]),
    (0x1000, mtimecmp_lo, rw, "the compare, low half"),
    (0x1001, mtimecmp_hi, rw, "the compare, high half"),
    (0x2ffe, mtime_lo, rw, "the count, low half"),
    (0x2fff, mtime_hi, rw, "the count, high half"),
] }
// end{map}

/// The word an address selects in the map. A word is addressed only
/// at its first byte: an address with either low bit set selects
/// word 1, which the map does not name, so it reads zero and takes no
/// write, as the reference model has it. A store of a byte or a half
/// to the second lane of a register is dropped, and a stock driver
/// only ever reads and writes whole words.
#[lower]
fn sel_of(addr: U<32>) -> U<14> {
    mux(
        addr.slice::<0, 2>() == 0,
        addr.slice::<2, 14>(),
        U::<14>::from(1u8),
    )
}

#[derive(Trace, Default)]
pub struct Timer<const I: usize> {
    pub mtime: Reg<U<64>>,
    pub mtimecmp: Reg<U<64>>,
    /// The software interrupt: bit 0 of the word at `msip`.
    pub msip: Reg<Bit>,
    pub pending: Reg<Bit>,
    /// A write taken and waiting for its beat: the write enables its
    /// word sets, decoded by the map when the request was taken, and
    /// which identifier answers it. Five bits rather than the
    /// fourteen of the word's select, which the beat does not need.
    pub pend: Reg<U<1>>,
    pub pwe: Reg<U<5>>,
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
            // The word a request selects in the map; the map answers zero
            // for a word it does not name, and no write enable is set for
            // one (issue 681).
            let sel = sel_of(q.addr);
            let msip = clint_msip_pack(self.msip.get());
            let (cmp_lo, cmp_hi) =
                (mtimecmp.slice::<0, 32>(), mtimecmp.slice::<32, 32>());
            let (time_lo, time_hi) =
                (mtime.slice::<0, 32>(), mtime.slice::<32, 32>());
            let word = clint_read(sel, msip, cmp_lo, cmp_hi, time_lo, time_hi);
            // The word a write's beat merges into, chosen by the enables
            // held since its request: at most one is set, and none for a
            // word the map does not name, which merges into zero and is
            // written nowhere.
            let pwe = self.pwe.get();
            let old = mux(
                pwe.bit(0),
                msip,
                mux(
                    pwe.bit(1),
                    cmp_lo,
                    mux(
                        pwe.bit(2),
                        cmp_hi,
                        mux(
                            pwe.bit(3),
                            time_lo,
                            mux(pwe.bit(4), time_hi, U::<32>::from(0u8)),
                        ),
                    ),
                ),
            );
            let we = mux(wgo, pwe, U::<5>::from(0u8));
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
                    pwe: clint_we(Bit::One, sel),
                    pid: q.id,
                },
                wgo ? pend: U::<1>::from(0u8),
                we.bit(0) ? msip: clint_msip_msip(merged),
                we.bit(1) ? mtimecmp: mtimecmp
                    .slice::<32, 32>()
                    .concat::<_, 64>(merged),
                we.bit(2) ? mtimecmp: merged
                    .concat::<_, 64>(mtimecmp.slice::<0, 32>()),
                we.bit(3) ?
                    mtime: mtime.slice::<32, 32>().concat::<_, 64>(merged),
                we.bit(4) ?
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
