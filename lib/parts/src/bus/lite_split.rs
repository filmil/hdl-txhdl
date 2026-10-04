// SPDX-License-Identifier: Apache-2.0
//! One AXI-Lite slot split in two by an address bit (issue 151).
//!
//! The board's peripheral page gives each peripheral one slot of 256
//! bytes, and a slot is one AXI-Lite link. [`LiteSplit`] puts two
//! peripherals in one slot: a transaction whose address has bit `BIT`
//! low goes to the first, `lo`, and one whose bit is high to the
//! second, `hi`. Neither peripheral sees the bit, since each decodes
//! only the low bits of its own words.
//!
//! One read and one write are in flight at a time, each remembered by
//! the side it went to, so an answer comes back from the side that owes
//! it and two answers never pass each other. That is what the host on
//! the other end, an AXI4-to-AXI-Lite bridge that also takes one at a
//! time, asks for, and it keeps the part free of a queue.
use crate::bus::axi_lite::{LiteAr, LiteAw, LiteB, LitePort, LiteR, LiteW};
use txhdl::comp::{mux, Clock, DefaultClock, Reg, Rx, Tx, Unit};
use txhdl::types::Bit;
use txhdl::{lower, with, Trace};

// begin{state}
/// An AXI-Lite slot split in two by address bit `BIT`. `A`, `D` and
/// `S` are the address, data and strobe widths.
#[derive(Trace, Default)]
pub struct LiteSplit<
    const A: usize,
    const D: usize,
    const S: usize,
    const BIT: usize,
> {
    /// A read is out, waiting for its word.
    pub rpend: Reg<Bit>,
    /// The side it went to: high for `hi`.
    pub rside: Reg<Bit>,
    /// A write is out, waiting for its response.
    pub wpend: Reg<Bit>,
    /// The side it went to.
    pub wside: Reg<Bit>,
}
// end{state}

// begin{run}
#[lower]
impl<const A: usize, const D: usize, const S: usize, const BIT: usize> Unit
    for LiteSplit<A, D, S, BIT>
{
    async fn run(
        &mut self,
        bus: LitePort<A, D, S>,
        (lo_aw, lo_ar, lo_w, lo_b, lo_r, hi_aw, hi_ar, hi_w, hi_b, hi_r): (
            Tx<LiteAw<A>>,
            Tx<LiteAr<A>>,
            Tx<LiteW<D, S>>,
            Rx<LiteB>,
            Rx<LiteR<D>>,
            Tx<LiteAw<A>>,
            Tx<LiteAr<A>>,
            Tx<LiteW<D, S>>,
            Rx<LiteB>,
            Rx<LiteR<D>>,
        ),
    ) {
        loop {
            DefaultClock::rising().await;
            // A read: taken when none is out and its side has room.
            let arh = bus.ar.head();
            let rhi = arh.addr.bit(BIT);
            let rroom = (lo_ar.ready() & !rhi) | (hi_ar.ready() & rhi);
            let rtake =
                Bit::from(bus.ar.peek().is_some()) & !self.rpend.get() & rroom;
            let _ = bus.ar.recv_if(rtake);
            // Its word: from the side it went to, when the host has room.
            let rs = self.rside.get();
            let lo_word = Bit::from(lo_r.peek().is_some()) & !rs;
            let hi_word = Bit::from(hi_r.peek().is_some()) & rs;
            let rback = self.rpend.get() & (lo_word | hi_word) & bus.r.ready();
            let lo_rh = lo_r.head();
            let hi_rh = hi_r.head();
            let _ = lo_r.recv_if(rback & !rs);
            let _ = hi_r.recv_if(rback & rs);
            // A write: the address and the word together, as the
            // peripherals behind take them.
            let awh = bus.aw.head();
            let wh = bus.w.head();
            let whi = awh.addr.bit(BIT);
            let wroom = (lo_aw.ready() & lo_w.ready() & !whi)
                | (hi_aw.ready() & hi_w.ready() & whi);
            let wtake = Bit::from(bus.aw.peek().is_some())
                & Bit::from(bus.w.peek().is_some())
                & !self.wpend.get()
                & wroom;
            let _ = bus.aw.recv_if(wtake);
            let _ = bus.w.recv_if(wtake);
            let ws = self.wside.get();
            let lo_resp = Bit::from(lo_b.peek().is_some()) & !ws;
            let hi_resp = Bit::from(hi_b.peek().is_some()) & ws;
            let wback = self.wpend.get() & (lo_resp | hi_resp) & bus.b.ready();
            let lo_bh = lo_b.head();
            let hi_bh = hi_b.head();
            let _ = lo_b.recv_if(wback & !ws);
            let _ = hi_b.recv_if(wback & ws);
            with!(self <= {
                rtake ? { rpend: Bit::One, rside: rhi },
                rback ? rpend: Bit::Zero,
                wtake ? { wpend: Bit::One, wside: whi },
                wback ? wpend: Bit::Zero,
            });
            if (rtake & !rhi).to_bool() {
                lo_ar.send(arh);
            }
            if (rtake & rhi).to_bool() {
                hi_ar.send(arh);
            }
            if rback.to_bool() {
                bus.r.send(mux(rs, hi_rh, lo_rh));
            }
            if (wtake & !whi).to_bool() {
                lo_aw.send(awh);
                lo_w.send(wh);
            }
            if (wtake & whi).to_bool() {
                hi_aw.send(awh);
                hi_w.send(wh);
            }
            if wback.to_bool() {
                bus.b.send(mux(ws, hi_bh, lo_bh));
            }
        }
    }
}
// end{run}
