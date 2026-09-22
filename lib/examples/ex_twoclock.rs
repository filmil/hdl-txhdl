// SPDX-License-Identifier: Apache-2.0
//! Two clocks in one netlist, checked against one run.
//!
//! A unit's process waits on one clock, so a design of two clocks is a
//! unit of units: one counter on the default clock, one on a clock
//! three times slower, and a parent that holds both and names neither.
//! Nothing crosses between them here. That is deliberate: what this
//! example is for is the checking rather than the crossing, and a
//! crossing would put the interesting part inside a `Crossing` rather
//! than at the ports where a testbench can see it.
//!
//! Until issue 131 the build could not check such a unit at all. The
//! generated testbench advanced in cycles of the first clock and
//! compared every output one of those cycles after driving it, so a
//! port on the slower clock was read before its own edge had produced
//! anything, and the two disagreed for part of every period. A port
//! carries its clock now, from the type where it is written to the
//! ports file the generator reads, and each port is checked against
//! its own clock's edge.
use txhdl::comp::trace::{stop, Wave};
use txhdl::comp::{
    join2, signal, Clock, DefaultClock, In, Out, Reg, Running, Unit,
};
use txhdl::types::{Bit, U};
use txhdl::{lower, with, Trace};

// begin{clock}
/// Six ticks a cycle where the default clock takes two: three times
/// slower, and out of phase with it on two edges of every three.
pub struct Slow;
impl Clock for Slow {
    const NAME: &'static str = "slow";
    const PERIOD: u64 = 6;
}
// end{clock}

// begin{units}
/// A counter on the default clock, which it never names.
#[derive(Trace, Default)]
pub struct Fast {
    pub n: Reg<U<8>>,
}

#[lower]
impl Unit for Fast {
    async fn run(&mut self, go: In<Bit>, a: Out<U<8>>) {
        loop {
            DefaultClock::rising().await;
            let n = self.n.get();
            a.set(n);
            with!(self <= { go.get() ? n: n + 1 });
        }
    }
}

/// The same on the slow clock, which it does name, in its register
/// and at both its ports.
#[derive(Trace, Default)]
pub struct SlowCount {
    pub n: Reg<U<8>, Slow>,
}

#[lower]
impl Unit for SlowCount {
    async fn run(&mut self, go: In<Bit, Slow>, b: Out<U<8>, Slow>) {
        loop {
            Slow::rising().await;
            let n = self.n.get();
            b.set(n);
            with!(self <= { go.get() ? n: n + 1 });
        }
    }
}

/// The two of them, and nothing else. The parent names no clock: it
/// holds units that each name their own.
///
/// The slow counter's field is `ticker` rather than `slow` because a
/// field and a clock of one name write an instance and a pin of one
/// name, which Verilator refuses and the lowering does not yet catch.
/// That is issue 367.
#[derive(Trace, Default)]
pub struct Two {
    pub fast: Fast,
    pub ticker: SlowCount,
}

#[lower]
impl Unit for Two {
    async fn run(
        &mut self,
        (go, go_slow): (In<Bit>, In<Bit, Slow>),
        (a, b): (Out<U<8>>, Out<U<8>, Slow>),
    ) {
        join2(self.fast.run(go, a), self.ticker.run(go_slow, b)).await;
    }
}
// end{units}

fn main() {
    let (go_o, go) = signal::<Bit, DefaultClock>();
    let (slow_o, go_slow) = signal::<Bit, Slow>();
    let (a_o, a) = signal::<U<8>, DefaultClock>();
    let (b_o, b) = signal::<U<8>, Slow>();
    let mut two = Two::default();

    if let Some(mut wave) = Wave::from_env() {
        wave.clock::<DefaultClock>();
        wave.clock::<Slow>();
        wave.add("go", &go);
        wave.add("go_slow", &go_slow);
        wave.add("a", &a);
        wave.add("b", &b);
        wave.add("two", &two);
        wave.start();
    }

    let mut sim = Running::new(two.run((go, go_slow), (a_o, b_o)));
    println!("   t  the fast count  the slow count");
    for tick in 0..60 {
        // The fast one is told to count from the third tick and stops
        // halfway, so neither port is a function of the other's clock
        // and a testbench that mixed them up would be caught. The slow
        // one is told on and off by turns, one tick after each of its
        // own edges, which come every third tick: what it counts is
        // what was in force at the edge, and a testbench that offered
        // it the value from after the edge is caught (issue 405).
        go_o.set(Bit::from_bool((3..30).contains(&tick)));
        slow_o.set(Bit::from_bool(tick >= 1 && ((tick - 1) / 3) % 2 == 0));
        sim.cycle();
        if tick % 6 == 0 {
            println!("{tick:4}  {:14}  {:14}", a.get().raw(), b.get().raw());
        }
    }
    stop();
    let net = Two::lowered("two");
    txhdl::netlist::write_vhdl_from_env(&net);
    print!("\n{}", net.verilog());
}
