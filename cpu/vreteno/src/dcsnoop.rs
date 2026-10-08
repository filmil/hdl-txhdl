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
//! Four bursts may be noted at once; a fifth waits at its address
//! phase. Each host's identifiers are its own and none is reused while
//! its burst is out, so a response is matched by its identifier alone.
use txhdl::comp::{mux, Clock, DefaultClock, Out, Reg, Rx, Tx, Unit};
use txhdl::types::{Bit, U};
use txhdl::{lower, with, Trace};
use txhdl_parts::bus::axi::{Aw, B, W};

#[derive(Trace, Default)]
pub struct DcSnoop {
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
impl Unit for DcSnoop {
    async fn run(
        &mut self,
        (aw_in, w_in, b_in): (Rx<Aw<32, 5>>, Rx<W<32, 4>>, Rx<B<5>>),
        (aw_out, w_out, b_out, inv): (
            Tx<Aw<32, 5>>,
            Tx<W<32, 4>>,
            Tx<B<5>>,
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
            let note = Bit::from(aw.id.slice::<2, 3>() != 0)
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
            let start = b_here & hit & !busy;
            let b_go = b_here & !hit & !busy & b_out.ready();
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
            inv.set(mux(
                walking,
                U::<1>::from(1u8).concat::<_, 9>(self.wline.get()),
                U::<9>::from(0u16),
            ));
            let e0 = last & Bit::from(wslot == 0);
            let e1 = last & Bit::from(wslot == 1);
            let e2 = last & Bit::from(wslot == 2);
            let e3 = last & Bit::from(wslot == 3);
            with!(self <= {
                v0: mux(s0, Bit::One, mux(e0, Bit::Zero, v0)),
                v1: mux(s1, Bit::One, mux(e1, Bit::Zero, v1)),
                v2: mux(s2, Bit::One, mux(e2, Bit::Zero, v2)),
                v3: mux(s3, Bit::One, mux(e3, Bit::Zero, v3)),
                s0 ? { id0: aw.id, line0: line, cnt0: cnt },
                s1 ? { id1: aw.id, line1: line, cnt1: cnt },
                s2 ? { id2: aw.id, line2: line, cnt2: cnt },
                s3 ? { id3: aw.id, line3: line, cnt3: cnt },
                start ? {
                    walking: Bit::One,
                    wline: first,
                    wleft: count,
                    wslot: slot
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
