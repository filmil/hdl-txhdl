// SPDX-License-Identifier: Apache-2.0
//! The cheat sheet's design: a counter that ticks an output once per
//! eight cycles while enabled. Small enough to show whole, and it
//! shows everything: a unit with a register, a wait, a read, a
//! predicated drive, an output, a lowering and a waveform.
use txhdl::comp::trace::{stop, Wave};
use txhdl::comp::{signal, Clock, DefaultClock, In, Out, Reg, Running, Unit};
use txhdl::types::{Bit, U};
use txhdl::{lower, when, Trace};

// begin{unit}
#[derive(Trace, Default)]
pub struct Counter {
    pub n: Reg<U<3>>,
}

#[lower]
impl Unit<In<Bit>, Out<Bit>> for Counter {
    async fn run(&mut self, enable: In<Bit>, tick: Out<Bit>) {
        loop {
            DefaultClock::rising().await; // wait, then read
            let n = self.n.get();
            let en = enable.get();
            when!(en => { self.n <= n + 1 });
            tick.set(n == 7);
        }
    }
}
// end{unit}

fn main() {
    let (en_out, enable) = signal::<Bit, DefaultClock>();
    let (tick_out, tick) = signal::<Bit, DefaultClock>();
    let mut counter = Counter::default();
    if let Some(mut w) = Wave::from_env() {
        w.clock::<DefaultClock>();
        w.add("enable", &enable);
        w.add("counter", &counter);
        w.add("tick", &tick);
        w.start();
    }
    let mut sim = Running::new(counter.run(enable, tick_out));
    for c in 0..12 {
        en_out.set(c < 10);
        sim.cycle();
    }
    stop();
    print!("{}", Counter::verilog("counter"));
    txhdl::netlist::write_vhdl_from_env(&Counter::lowered("counter"));
}
