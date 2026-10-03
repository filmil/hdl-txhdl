// SPDX-License-Identifier: Apache-2.0
//! A unit of two clocks whose first child is on the slower one.
//!
//! The ports file lists a netlist's clocks in the order the lowering
//! meets them, so here `slow` comes before `clk`. A port on the default
//! clock carries no mark there, and the testbench generator once read
//! an unmarked port as on the first clock listed: `go` and `a`, on
//! `clk`, were applied and checked at `slow`'s edges, and the netlist
//! was told `go` for three cycles where the run had it for one (issue
//! 888). An unmarked port is on `clk` now, whatever the order.
//!
//! `go` lets the fast counter count for one cycle, between two edges
//! of `slow`, which is where reading it at the wrong clock shows: the
//! netlist then counted three where the run counted one.
use txhdl::comp::trace::{stop, Wave};
use txhdl::comp::{
    join2, signal, Clock, DefaultClock, In, Out, Reg, Running, Unit,
};
use txhdl::types::{Bit, U};
use txhdl::{lower, with, Trace};

/// Six ticks a cycle where the default clock takes two.
pub struct Slow;
impl Clock for Slow {
    const NAME: &'static str = "slow";
    const PERIOD: u64 = 6;
}

// begin{units}
/// A counter on the default clock, counting while `go` is high.
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

/// A counter on the slow clock, counting while `go` is high.
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

/// The two, the slow one first, so the netlist meets `slow` first.
#[derive(Trace, Default)]
pub struct SlowFirst {
    pub ticker: SlowCount,
    pub fast: Fast,
}

#[lower]
impl Unit for SlowFirst {
    async fn run(
        &mut self,
        (go, go_slow): (In<Bit>, In<Bit, Slow>),
        (a, b): (Out<U<8>>, Out<U<8>, Slow>),
    ) {
        join2(self.ticker.run(go_slow, b), self.fast.run(go, a)).await;
    }
}
// end{units}

fn main() {
    let (go_o, go) = signal::<Bit, DefaultClock>();
    let (slow_o, go_slow) = signal::<Bit, Slow>();
    let (a_o, a) = signal::<U<8>, DefaultClock>();
    let (b_o, b) = signal::<U<8>, Slow>();
    let mut top = SlowFirst::default();

    if let Some(mut wave) = Wave::from_env() {
        wave.clock::<DefaultClock>();
        wave.clock::<Slow>();
        wave.add("go", &go);
        wave.add("go_slow", &go_slow);
        wave.add("a", &a);
        wave.add("b", &b);
        wave.add("slowfirst", &top);
        wave.start();
    }

    let mut sim = Running::new(top.run((go, go_slow), (a_o, b_o)));
    // `go` for one cycle, the tenth, at tick 18, between `slow`'s edges
    // at 12 and 18 and 24; `go_slow` from the second cycle on.
    for cycle in 0..24 {
        go_o.set(Bit::from_bool(cycle == 9));
        slow_o.set(Bit::from_bool(cycle >= 1));
        sim.cycle();
    }
    let (fast, slow) = (a.get().raw(), b.get().raw());
    println!("a = {fast}, b = {slow}");
    stop();
    let net = SlowFirst::lowered("slowfirst");
    print!("\n{}", net.ports_file());
    txhdl::netlist::write_vhdl_from_env(&net);
}
