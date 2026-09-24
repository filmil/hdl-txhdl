// SPDX-License-Identifier: Apache-2.0
//! General purpose pins on AXI-Lite: a value out, a value in, a
//! direction, and an interrupt per pin.
//!
//! The part drives no pin itself. It offers the three wires a pad
//! wants, the value to drive, the direction that says whether to drive
//! it, and the value read back, and a board's top level joins them to
//! a tristate buffer. That keeps the part something a simulation can
//! run: a pad carries nothing until a netlist exists, and a peripheral
//! nobody can simulate is a peripheral nobody can check.
//!
//! An input arrives through two flip-flops before anything reads it.
//! A pin is a wire to the outside and its edges fall where they like,
//! so a value taken straight into logic is a value that can be half
//! way between one and zero when two gates read it. Two flip-flops is
//! the usual price of not caring.
//!
//! The seven registers, a word apart from the base:
//!
//! | Offset | Name | What it is |
//! |---|---|---|
//! | `0x00` | `dout` | driven on a pin whose direction is out |
//! | `0x04` | `in` | what the pins read, after the two flip-flops |
//! | `0x08` | `dir` | one to drive the pin, zero to read it |
//! | `0x0c` | `ie` | one to let the pin raise the interrupt |
//! | `0x10` | `kind` | zero for a level, one for an edge |
//! | `0x14` | `pol` | a level: one for high; an edge: one for rising |
//! | `0x18` | `status` | which pins have fired; write one to clear |
//!
//! `status` is sticky and is cleared by writing a one to the bit, not
//! a zero, so two programs clearing different pins cannot lose each
//! other's. A pin set for a level sets its bit again as soon as it is
//! cleared while the level lasts, which is what a level means.
use txhdl::comp::{mux, Clock, DefaultClock, In, Out, Reg, Unit};
use txhdl::types::{Bit, U};
use txhdl::{lower, select, with, Trace};

use crate::bus::axi::Resp;
use crate::bus::axi_lite::{LiteB, LitePort, LiteR};

// begin{state}
/// `N` pins on an AXI-Lite link.
///
/// `N` is how many pins there are, and every register is that wide;
/// bits above `N` read as zero. The synchroniser is `sync0` and
/// `sync1`, and `seen` is what `sync1` held a cycle ago, which is what
/// an edge is measured against.
#[derive(Trace, Default)]
pub struct Gpio<const N: usize> {
    /// What to drive on a pin whose direction is out. Named `dout`
    /// and not `out` because `out` is a reserved word in VHDL and the
    /// lowering neither escapes nor rejects one, which is #77.
    pub dout: Reg<U<N>>,
    /// One to drive the pin, zero to read it.
    pub dir: Reg<U<N>>,
    /// The pins as they arrived, one flip-flop in.
    pub sync0: Reg<U<N>>,
    /// The pins as anything else may read them.
    pub sync1: Reg<U<N>>,
    /// What `sync1` held a cycle ago.
    pub seen: Reg<U<N>>,
    /// One to let the pin raise the interrupt.
    pub ie: Reg<U<N>>,
    /// Zero for a level, one for an edge.
    pub kind: Reg<U<N>>,
    /// A level: one for high. An edge: one for rising.
    pub pol: Reg<U<N>>,
    /// Which pins have fired, until written with a one.
    pub status: Reg<U<N>>,
}
// end{state}

// begin{run}
#[lower]
impl<const N: usize> Unit for Gpio<N> {
    async fn run(
        &mut self,
        bus: LitePort<32, 32, 4>,
        (pins, drive, dirs, irq): (In<U<N>>, Out<U<N>>, Out<U<N>>, Out<Bit>),
    ) {
        loop {
            DefaultClock::rising().await;
            let dout = self.dout.get();
            let dir = self.dir.get();
            let sync1 = self.sync1.get();
            let seen = self.seen.get();
            let ie = self.ie.get();
            let kind = self.kind.get();
            let pol = self.pol.get();
            let status = self.status.get();
            // What has happened at each pin. A level fires while the
            // pin equals the polarity; an edge fires on the one
            // crossing the polarity names. `crossed` is not called
            // `edge` because that is a reserved word in Verilog and
            // the lowering neither escapes nor rejects one, #77.
            let level = !(sync1 ^ pol);
            let rose = sync1 & !seen;
            let fell = !sync1 & seen;
            let crossed = (pol & rose) | (!pol & fell);
            let fired = ((kind & crossed) | (!kind & level)) & ie;
            // The bus. A read is answered in the cycle it is taken; a
            // write needs its beat and room for its response.
            let arh = bus.ar.head();
            let awh = bus.aw.head();
            let wh = bus.w.head();
            let rsel = arh.addr.slice::<2, 3>();
            let wsel = awh.addr.slice::<2, 3>();
            let rgo = bus.r.ready() & bus.ar.peek().is_some();
            let _ = bus.ar.recv_if(bus.r.ready());
            let wgo = bus.b.ready()
                & bus.aw.peek().is_some()
                & bus.w.peek().is_some();
            let _ = bus.aw.recv_if(wgo);
            let _ = bus.w.recv_if(wgo);
            let written = wh.data.slice::<0, N>();
            let word = select!(rsel.raw() => {
                0 => dout.zext::<32>(),
                1 => sync1.zext::<32>(),
                2 => dir.zext::<32>(),
                3 => ie.zext::<32>(),
                4 => kind.zext::<32>(),
                5 => pol.zext::<32>(),
                6 => status.zext::<32>(),
                _ => U::<32>::from(0u8),
            });
            // A one written to `status` clears that bit, and a pin
            // firing in the same cycle sets it again.
            let cleared = mux(wgo & (wsel == 6), status & !written, status);
            with!(self <= {
                sync0: pins.get(),
                sync1: self.sync0.get(),
                seen: sync1,
                status: cleared | fired,
                wgo & (wsel == 0) ? dout: written,
                wgo & (wsel == 2) ? dir: written,
                wgo & (wsel == 3) ? ie: written,
                wgo & (wsel == 4) ? kind: written,
                wgo & (wsel == 5) ? pol: written,
            });
            if rgo.to_bool() {
                bus.r.send(LiteR {
                    data: word,
                    resp: Resp::Okay,
                });
            }
            if wgo.to_bool() {
                bus.b.send(LiteB { resp: Resp::Okay });
            }
            drive.set(dout);
            dirs.set(dir);
            irq.set(status != U::<N>::from(0u8));
        }
    }
}
// end{run}
