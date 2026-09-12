// SPDX-License-Identifier: Apache-2.0
//! An internal signal, seen. A `let` inside the loop is a wire in the
//! netlist but nothing in a waveform, since it lives only in the step.
//! `Wire<T>` is a `let` kept as a field: the process drives it with
//! `set`, may read it back in the same step, and it shows in the trace
//! under the unit's name like a register, while in the netlist it is
//! the wire the `let` would have been. The unit is a divider by three
//! that keeps its two internal decisions in wires; the testbench never
//! sees them through a port, and the waveform shows them anyway. The
//! netlist is checked against the trace under nvc and Verilator, wires
//! included.
use txhdl::comp::trace::{stop, Wave};
use txhdl::comp::{
    signal, Clock, DefaultClock, In, Out, Reg, Running, Unit, Wire,
};
use txhdl::funcs::eq;
use txhdl::types::{Bit, U};
use txhdl::{lower, when, Trace};

// begin{unit}
#[derive(Trace, Default)]
pub struct Divider {
    pub n: Reg<U<2>>,
    /// Internal, and visible: whether the count wraps this cycle,
    /// and whether the input is being counted at all.
    pub wrap: Wire<Bit>,
    pub counting: Wire<Bit>,
}

#[lower]
impl Unit<In<Bit>, Out<Bit>> for Divider {
    async fn run(&mut self, pulse: In<Bit>, tick: Out<Bit>) {
        loop {
            DefaultClock::rising().await;
            let n = self.n.get();
            self.counting.set(pulse.get());
            self.wrap.set(self.counting.get().and(eq(n, U::from(2u8))));
            let (counting, wrap) = (self.counting.get(), self.wrap.get());
            when!(wrap => { self.n <= U::from(0u8) });
            when!(counting.and(wrap.not()) => { self.n <= n.wrapping_add(1) });
            tick.set(wrap);
        }
    }
}
// end{unit}

fn main() {
    let (pulse_out, pulse) = signal::<Bit, DefaultClock>();
    let (tick_out, tick) = signal::<Bit, DefaultClock>();
    let mut div = Divider::default();
    if let Some(mut w) = Wave::from_env() {
        w.clock::<DefaultClock>();
        w.add("pulse", &pulse);
        w.add("div", &div);
        w.add("tick", &tick);
        w.start();
    }
    let mut sim = Running::new(div.run(pulse, tick_out));
    // Pulses on all but every fourth cycle: the divider sees nine of
    // them in twelve cycles and wraps three times.
    for c in 0..12 {
        pulse_out.set(Bit::from_bool(c % 4 != 3));
        sim.cycle();
        println!(
            "t={:>2} pulse {} tick {}",
            txhdl::comp::now(),
            (c % 4 != 3) as u8,
            tick.get().to_bool() as u8
        );
    }
    stop();
    txhdl::netlist::write_vhdl_from_env(&Divider::lowered("divider"));
    print!("\n{}", Divider::verilog("divider"));
}
