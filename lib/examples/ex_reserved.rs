// SPDX-License-Identifier: Apache-2.0
//! Names VHDL or SystemVerilog reserve, lowered as they are written.
//!
//! Rust reserves its own words and not theirs, so a port may be called
//! `next`, `begin` or `out`, and a register `signal`, and each of those
//! is a word one netlist or the other cannot declare. The lowering
//! escapes such a name rather than refusing it (issue 497): the netlist
//! writes it with `_rw` after it, the same in VHDL and in
//! SystemVerilog, so `next` is `next_rw` in both.
//!
//! The run is checked against the netlist under nvc and Verilator, and
//! that is the point of the example. The testbench reads each port's
//! values from the trace by name, and a port whose name differs between
//! the netlist and the trace would be bound to nothing, or to the wrong
//! signal, without a word (issue 462). So an escaped port is recorded
//! with its own name as its trace scope, and a register is traced under
//! its escaped name, and the co-simulation binds all four.
use txhdl::comp::trace::{stop, Wave};
use txhdl::comp::{
    now, signal, Clock, DefaultClock, In, Out, Reg, Running, Unit,
};
use txhdl::types::{Bit, U};
use txhdl::{lower, when, Trace};

// begin{unit}
/// A latch: `begin` loads `next` into `signal`, and `out` is what it
/// holds against what is offered.
#[derive(Trace, Default)]
pub struct Latch {
    /// VHDL reserves `signal`: the register is `signal_rw`.
    pub signal: Reg<U<8>>,
}

#[lower]
impl Unit for Latch {
    async fn run(
        &mut self,
        // VHDL reserves `next`, and both targets reserve `begin`.
        (next, begin): (In<U<8>>, In<Bit>),
        // VHDL reserves `out`.
        out: Out<U<8>>,
    ) {
        loop {
            DefaultClock::rising().await;
            when!(begin.get() => self { signal: next.get() });
            out.set(self.signal.get() ^ next.get());
        }
    }
}
// end{unit}

fn main() {
    let (next_out, next) = signal::<U<8>, DefaultClock>();
    let (begin_out, begin) = signal::<Bit, DefaultClock>();
    let (out_out, out) = signal::<U<8>, DefaultClock>();
    let mut latch = Latch::default();
    let held = latch.signal;
    if let Some(mut w) = Wave::from_env() {
        w.clock::<DefaultClock>();
        // The ports are traced under the names `run` gives them.
        w.add("next", &next);
        w.add("begin", &begin);
        w.add("latch", &latch);
        w.add("out", &out);
        w.start();
    }
    let mut sim = Running::new(latch.run((next, begin), out_out));
    for c in 0..8u32 {
        next_out.set(U::<8>::from((c * 37 + 5) as u8));
        begin_out.set(c % 3 == 1);
        sim.cycle();
        println!(
            "t={:>2} signal {:>3} out {:>3}",
            now(),
            held.get().raw(),
            out.get().raw()
        );
    }
    stop();
    let net = Latch::lowered("latch");
    txhdl::netlist::write_vhdl_from_env(&net);
    print!("\n{}", net.verilog());
}
