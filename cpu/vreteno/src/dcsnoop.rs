// SPDX-License-Identifier: Apache-2.0
//! The data cache's snoop (issue 1275): what keeps the core's cache of
//! the DDR3 true when another host writes the DDR3.
//!
//! It stands on the router's port to the DDR3, and passes the write
//! address and data through as they come.
//! A write burst from a host other than the core, into the DDR3, is
//! noted at its address phase: its identifier, the first line of the
//! cache it falls in and how many lines it covers. When the burst's
//! response comes back, which is when the controller has made the
//! write good, the snoop tells the core one line a cycle to invalidate,
//! and holds the response until the last is told and a cycle more, so
//! that a host's "done" means the core's copy is gone. A line is named
//! by its index alone, which may take out a line of another page:
//! harmless, and it keeps the compare out of the snoop.
//!
//! With a second hart on port `P1` (issue 1408), every write into the
//! DDR3 is noted, the first hart's too, and each line is named to each
//! hart that did not write it: on `inv` unless the writer was port 0,
//! on `inv1` unless it was `P1`. With `P1` zero there is one hart,
//! its own writes are not noted, and `inv1` names nothing.
//!
//! Four bursts may be noted at once; a fifth waits at its address
//! phase. Each host's identifiers are its own and none is reused while
//! its burst is out, so a response is matched by its identifier alone.
use txhdl::comp::{mux, Clock, DefaultClock, In, Out, Reg, Rx, Tx, Unit};
use txhdl::types::{Bit, U};
use txhdl::{lower, with, Trace};
use txhdl_parts::bus::axi::{Aw, B, W};

#[derive(Trace, Default)]
pub struct DcSnoop<const P1: usize = 0> {
    /// The four bursts noted: whether each slot holds one, its
    /// identifier, its first line and its count of lines.
    pub v0: Reg<Bit>,
    pub v1: Reg<Bit>,
    pub v2: Reg<Bit>,
    pub v3: Reg<Bit>,
    pub id0: Reg<U<5>>,
    pub id1: Reg<U<5>>,
    pub id2: Reg<U<5>>,
    pub id3: Reg<U<5>>,
    pub line0: Reg<U<8>>,
    pub line1: Reg<U<8>>,
    pub line2: Reg<U<8>>,
    pub line3: Reg<U<8>>,
    pub cnt0: Reg<U<7>>,
    pub cnt1: Reg<U<7>>,
    pub cnt2: Reg<U<7>>,
    pub cnt3: Reg<U<7>>,
    /// The port each noted burst came from, and the walk's.
    pub pt0: Reg<U<3>>,
    pub pt1: Reg<U<3>>,
    pub pt2: Reg<U<3>>,
    pub pt3: Reg<U<3>>,
    pub wport: Reg<U<3>>,
    /// Whether the walk's lines go to the second hart too.
    pub wtell1: Reg<Bit>,
    /// The walk over a burst's lines: whether one is on, the next line,
    /// how many are left, and the slot it empties at the end; and the
    /// cycle after it, in which the response still waits.
    pub walking: Reg<Bit>,
    pub wline: Reg<U<8>>,
    pub wleft: Reg<U<7>>,
    pub wslot: Reg<U<2>>,
    pub settle: Reg<Bit>,
}

#[lower]
impl<const P1: usize> Unit for DcSnoop<P1> {
    async fn run(
        &mut self,
        (aw_in, w_in, b_in, asleep1): (
            Rx<Aw<32, 5>>,
            Rx<W<32, 4>>,
            Rx<B<5>>,
            In<Bit>,
        ),
        (aw_out, w_out, b_out, inv, inv1): (
            Tx<Aw<32, 5>>,
            Tx<W<32, 4>>,
            Tx<B<5>>,
            Out<U<9>>,
            Out<U<9>>,
        ),
    ) {
        loop {
            DefaultClock::rising().await;
            let (v0, v1, v2, v3) =
                (self.v0.get(), self.v1.get(), self.v2.get(), self.v3.get());
            // A write address: passed on, and noted when it is another
            // host's into the DDR3, which waits while the table is full.
            // The arbiter puts the host's port above its own two bits of
            // identifier, and the core is port 0.
            let aw = aw_in.head();
            let port = aw.id.slice::<2, 3>();
            let note = (Bit::from(port != 0) | Bit::from(P1 != 0))
                & Bit::from(aw.addr.slice::<30, 2>() == 1);
            let full = v0 & v1 & v2 & v3;
            let aw_go =
                aw_in.peek().is_some() & aw_out.ready() & !(note & full);
            let ins = aw_go & note;
            let s0 = ins & !v0;
            let s1 = ins & v0 & !v1;
            let s2 = ins & v0 & v1 & !v2;
            let s3 = ins & v0 & v1 & v2 & !v3;
            let line = aw.addr.slice::<4, 8>();
            let span = aw.addr.slice::<2, 2>().zext::<9>() + aw.len.zext::<9>();
            let cnt = span.slice::<2, 7>() + 1;
            let _ = aw_in.recv_if(aw_go);
            if aw_go.to_bool() {
                aw_out.send(aw);
            }
            // The write data, passed on.
            let w = w_in.head();
            let w_go = w_in.peek().is_some() & w_out.ready();
            let _ = w_in.recv_if(w_go);
            if w_go.to_bool() {
                w_out.send(w);
            }
            // A response: one for a noted burst starts the walk over its
            // lines and waits for it; any other passes.
            let b = b_in.head();
            let b_here = b_in.peek().is_some();
            let m0 = v0 & Bit::from(self.id0.get() == b.id);
            let m1 = v1 & Bit::from(self.id1.get() == b.id);
            let m2 = v2 & Bit::from(self.id2.get() == b.id);
            let m3 = v3 & Bit::from(self.id3.get() == b.id);
            let hit = m0 | m1 | m2 | m3;
            let walking = self.walking.get();
            let busy = walking | self.settle;
            let from = mux(
                m0,
                self.pt0.get(),
                mux(
                    m1,
                    self.pt1.get(),
                    mux(m2, self.pt2.get(), self.pt3.get()),
                ),
            );
            // Whom a noted write's lines go to, decided as its answer
            // comes, when it has landed: the first hart unless it wrote
            // them, the second unless it wrote them or waits in `wfi`,
            // which forgets its cache as it wakes (issue 1408). A write
            // nobody is to be told of is answered at once.
            let tell0 = Bit::from(from != 0);
            let tell1 =
                Bit::from(from != P1) & Bit::from(P1 != 0) & !asleep1.get();
            let need = tell0 | tell1;
            let start = b_here & hit & need & !busy;
            let b_go = b_here & (!hit | !need) & !busy & b_out.ready();
            let skip = b_go & hit;
            let _ = b_in.recv_if(b_go);
            if b_go.to_bool() {
                b_out.send(b);
            }
            let first = mux(
                m0,
                self.line0.get(),
                mux(
                    m1,
                    self.line1.get(),
                    mux(m2, self.line2.get(), self.line3.get()),
                ),
            );
            let count = mux(
                m0,
                self.cnt0.get(),
                mux(
                    m1,
                    self.cnt1.get(),
                    mux(m2, self.cnt2.get(), self.cnt3.get()),
                ),
            );
            let slot = mux(
                m0,
                U::<2>::from(0u8),
                mux(
                    m1,
                    U::<2>::from(1u8),
                    mux(m2, U::<2>::from(2u8), U::<2>::from(3u8)),
                ),
            );
            // The walk: a line a cycle to the core, and the slot emptied
            // with the last.
            let wslot = self.wslot.get();
            let last = walking & Bit::from(self.wleft.get() == 1);
            // Each hart is told of the lines the other hosts wrote.
            let wport = self.wport.get();
            let named = U::<1>::from(1u8).concat::<_, 9>(self.wline.get());
            let none = U::<9>::from(0u16);
            inv.set(mux(walking & Bit::from(wport != 0), named, none));
            inv1.set(mux(walking & self.wtell1, named, none));
            let e0 = (last & Bit::from(wslot == 0)) | (skip & m0);
            let e1 = (last & Bit::from(wslot == 1)) | (skip & m1);
            let e2 = (last & Bit::from(wslot == 2)) | (skip & m2);
            let e3 = (last & Bit::from(wslot == 3)) | (skip & m3);
            with!(self <= {
                v0: mux(s0, Bit::One, mux(e0, Bit::Zero, v0)),
                v1: mux(s1, Bit::One, mux(e1, Bit::Zero, v1)),
                v2: mux(s2, Bit::One, mux(e2, Bit::Zero, v2)),
                v3: mux(s3, Bit::One, mux(e3, Bit::Zero, v3)),
                s0 ? { id0: aw.id, line0: line, cnt0: cnt, pt0: port },
                s1 ? { id1: aw.id, line1: line, cnt1: cnt, pt1: port },
                s2 ? { id2: aw.id, line2: line, cnt2: cnt, pt2: port },
                s3 ? { id3: aw.id, line3: line, cnt3: cnt, pt3: port },
                start ? {
                    walking: Bit::One,
                    wline: first,
                    wleft: count,
                    wslot: slot,
                    wport: from,
                    wtell1: tell1
                },
                walking ? {
                    wline: self.wline.get() + 1,
                    wleft: self.wleft.get() - 1
                },
                last ? walking: Bit::Zero,
                settle: last,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::DcSnoop;
    use txhdl::comp::{chan, signal, DefaultClock, Running, Unit};
    use txhdl::types::{Bit, U};
    use txhdl_parts::bus::axi::{Addr, Resp, B, W};

    /// With a second hart on port 7 (issue 1408), ports 0, 7 and 3 each
    /// write one word of a line of their own: hart 0 is told of the
    /// lines ports 7 and 3 wrote, hart 1 of those ports 0 and 3 wrote,
    /// and each answer passes once its lines are told.
    /// Ports 0, 7 and 3 each write one word of a line of their own,
    /// with hart 1 awake or waiting: the lines told to each hart, the
    /// answers that passed, and the cycle port 0's answer did.
    fn run(asleep: bool) -> (Vec<u128>, Vec<u128>, usize, usize) {
        let mut s = DcSnoop::<7>::default();
        let (sleep_o, sleep) = signal::<Bit, DefaultClock>();
        sleep_o.set(Bit::from(asleep));
        let (aw_tx, aw_in) = chan::<Addr<32, 5>, DefaultClock>();
        let (w_tx, w_in) = chan::<W<32, 4>, DefaultClock>();
        let (b_tx, b_in) = chan::<B<5>, DefaultClock>();
        let (aw_out, aw_rx) = chan();
        let (w_out, w_rx) = chan();
        let (b_out, b_rx) = chan();
        let (inv, inv_rx) = signal::<U<9>, DefaultClock>();
        let (inv1, inv1_rx) = signal::<U<9>, DefaultClock>();
        let mut sim = Running::new(s.run(
            (aw_in, w_in, b_in, sleep),
            (aw_out, w_out, b_out, inv, inv1),
        ));
        let (mut told0, mut told1, mut answers) = (vec![], vec![], 0);
        let mut port0_at = 0;
        for (port, line) in [(0u8, 0x10u32), (7, 0x20), (3, 0x30)] {
            let id = U::<5>::from(port << 2);
            aw_tx.send(Addr {
                id,
                addr: U::from(0x4000_0000 + line * 16),
                ..Addr::default()
            });
            w_tx.send(W {
                data: U::from(1u32),
                strb: U::from(0xfu8),
                last: Bit::One,
            });
            for c in 0..12 {
                sim.cycle();
                // The DDR3: it answers a write once its address passed.
                if aw_rx.recv().is_some() {
                    b_tx.send(B {
                        id,
                        resp: Resp::Okay,
                    });
                }
                let _ = w_rx.recv();
                if b_rx.recv().is_some() {
                    answers += 1;
                    if port == 0 {
                        port0_at = c;
                    }
                }
                let (a, b) = (inv_rx.get(), inv1_rx.get());
                if a.bit(8).to_bool() {
                    told0.push(a.slice::<0, 8>().raw());
                }
                if b.bit(8).to_bool() {
                    told1.push(b.slice::<0, 8>().raw());
                }
            }
        }
        (told0, told1, answers, port0_at)
    }

    /// With a second hart on port 7 (issue 1408), awake: hart 0 is told
    /// of the lines ports 7 and 3 wrote, hart 1 of those ports 0 and 3
    /// wrote, and each answer passes once its lines are told.
    #[test]
    fn each_hart_is_told_of_the_others_writes() {
        let (told0, told1, answers, _) = run(false);
        assert_eq!(told0, vec![0x20, 0x30], "hart 0: ports 7 and 3");
        assert_eq!(told1, vec![0x10, 0x30], "hart 1: ports 0 and 3");
        assert_eq!(answers, 3, "every answer passed");
    }

    /// With hart 1 waiting in `wfi`, it is told of nothing, which it
    /// makes up for by forgetting its cache as it wakes; port 0's write
    /// then has nobody to tell, and its answer is not held for a walk.
    #[test]
    fn a_waiting_hart_is_told_nothing() {
        let (told0, told1, answers, asleep_at) = run(true);
        let (_, _, _, awake_at) = run(false);
        assert_eq!(told0, vec![0x20, 0x30], "hart 0 as before");
        assert_eq!(told1, Vec::<u128>::new(), "hart 1 nothing");
        assert_eq!(answers, 3, "every answer passed");
        assert!(
            asleep_at < awake_at,
            "port 0's answer at {asleep_at}, against {awake_at} awake"
        );
    }
}
