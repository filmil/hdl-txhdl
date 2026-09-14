// SPDX-License-Identifier: Apache-2.0
//! A waveform. A signal is named by the field it lives in, the
//! hierarchy is the nesting of units, and the testbench names what it
//! holds itself. `#[derive(Trace)]` registers a unit's fields;
//! `#[derive(Value)]` says how a value of the design's own looks in a
//! trace. The sink is VCD, written here to stdout, which Surfer opens.
//! Time is in ticks, two per cycle of the default clock.
use txhdl::comp::trace::Vcd;
use txhdl::comp::{mux, signal, Clock, DefaultClock, Out, Reg, Running, Unit};
use txhdl::types::{Bit, U};
use txhdl::{Trace, Value};

/// A value of the design's own: two variants, so one bit in the trace.
#[derive(Value, Clone, Copy, Default, PartialEq, Debug)]
pub enum Phase {
    #[default]
    Low,
    High,
}

/// Counts, and says which half of the count it is in.
#[derive(Trace, Default)]
pub struct Counter {
    pub n: Reg<U<3>>,
    pub phase: Reg<Phase>,
}

impl Unit<(), Out<Bit>> for Counter {
    async fn run(&mut self, _i: (), msb: Out<Bit>) {
        loop {
            DefaultClock::rising().await;
            let high = self.n.get().bit(2);
            self.n.set(self.n + 1);
            self.phase.set(mux(high, Phase::High, Phase::Low));
            msb.set(high);
        }
    }
}

fn main() {
    let (drive, msb) = signal::<Bit, DefaultClock>();
    let mut counter = Counter::default();

    // Name what to watch, then start; every step from here on is written.
    let mut vcd = Vcd::new(std::io::stdout());
    vcd.clock::<DefaultClock>();
    vcd.add("counter", &counter);
    vcd.add("msb", &msb);
    vcd.start();

    let mut sim = Running::new(counter.run((), drive));
    for _ in 0..10 {
        sim.cycle();
    }
    txhdl::comp::trace::stop();
}
