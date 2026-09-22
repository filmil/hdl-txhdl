// SPDX-License-Identifier: Apache-2.0
//! A channel from one clock to another, lowered and checked.
//!
//! The producer runs on `ClkW`, period four, and the consumer on
//! `ClkR`, period six and a phase of two, so the two edges drift past
//! each other and every relative position happens somewhere in the
//! run. Between them is `ChanCdc` from `//lib/parts`: two processes,
//! one per clock, a memory written by one and read by the other, and
//! each side keeping the other's Gray pointer through two flops of
//! its own clock.
//!
//! Nothing here is hand-written Verilog. The netlist is what
//! `#[lower]` makes of the part, and the build checks it against this
//! run under nvc and under Verilator, at the ports and at every
//! register: the two Gray pointers and all four synchroniser flops
//! are compared tick by tick, so a pointer that advanced at the wrong
//! moment, or a full test off by the wrap bit, is a failure and not a
//! difference nobody looks at.
//!
//! What that does not prove is the part the flops are for. Both sides
//! are deterministic here and the sampling clock reads a register
//! that has settled, so metastability never happens and cannot be
//! made to. The agreement this run demonstrates is about the pointer
//! protocol; the second flop is an argument about silicon, and no
//! simulation of either language says anything about it.
use txhdl::comp::trace::{stop, Wave};
use txhdl::comp::{chan, join2, now, until, Clock, Reg, Running, Rx, Tx, Unit};
use txhdl::types::U;
use txhdl_parts::cdc::ChanCdc;

/// The writing clock: an edge every four ticks.
pub struct ClkW;
impl Clock for ClkW {
    const NAME: &'static str = "clk_w";
    const PERIOD: u64 = 4;
}

/// The reading clock: an edge every six, starting two in, so that it
/// never shares a phase with the writer for long.
pub struct ClkR;
impl Clock for ClkR {
    const NAME: &'static str = "clk_r";
    const PERIOD: u64 = 6;
    const PHASE: u64 = 2;
}

/// Sixteen words of eight bits, so the pointers are five.
pub type Cdc = ChanCdc<U<8>, 4, 16, 5, ClkW, ClkR>;

/// Offers a counting sequence on the writing clock, as fast as the
/// channel will take it.
#[derive(Default)]
pub struct Source {
    pub n: Reg<U<8>, ClkW>,
}

impl Unit<(), Tx<U<8>, ClkW>> for Source {
    async fn run(&mut self, _i: (), out: Tx<U<8>, ClkW>) {
        loop {
            ClkW::rising().await;
            if out.ready().to_bool() {
                out.send(self.n);
                self.n.set(self.n + 1);
            }
        }
    }
}

/// Takes words on the reading clock, and prints what it took.
#[derive(Default)]
pub struct Sink {
    pub count: Reg<U<8>, ClkR>,
}

impl Unit<Rx<U<8>, ClkR>, ()> for Sink {
    async fn run(&mut self, inp: Rx<U<8>, ClkR>, _o: ()) {
        loop {
            until(ClkR::rising, || inp.peek().is_some()).await;
            let v = inp.recv().unwrap_or_default();
            self.count.set(self.count + 1);
            println!("t={:>3} took {}", now(), v.raw());
        }
    }
}

fn main() {
    let (a_tx, a_rx) = chan::<U<8>, ClkW>();
    let (b_tx, b_rx) = chan::<U<8>, ClkR>();
    let mut source = Source::default();
    let mut cdc = Cdc::default();
    let mut sink = Sink::default();

    if let Some(mut wave) = Wave::from_env() {
        wave.clock::<ClkW>();
        wave.clock::<ClkR>();
        wave.add("inp", &a_rx);
        wave.add("out", &b_rx);
        wave.add("cdc", &cdc);
        wave.start();
    }

    let mut sim = Running::new(join2(
        join2(source.run((), a_tx), cdc.run(a_rx, b_tx)),
        sink.run(b_rx, ()),
    ));
    for _ in 0..40 {
        sim.cycle();
    }

    print!("\n{}", Cdc::verilog("cdc"));
    stop();
    txhdl::netlist::write_vhdl_from_env(&Cdc::lowered("cdc"));
}
