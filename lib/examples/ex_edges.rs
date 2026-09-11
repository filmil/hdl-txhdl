// SPDX-License-Identifier: Apache-2.0
//! Both edges. A clock has a rising edge and, half a period later, a
//! falling one, and a process may wait for either: `C::rising()`,
//! `C::falling()`, or `until(C::falling, || cond)` for a condition
//! asked at each falling edge. One process drives a count on the
//! rising edge; another captures it on the falling edge, half a cycle
//! later; a third waits at falling edges until the count reaches four
//! and says so once. The waveform shows the capture trailing the
//! count by half a cycle.
use txhdl::comp::trace::Vcd;
use txhdl::comp::{
    join2, now, until, Clock, DefaultClock, Module, Reg, Running,
};
use txhdl::types::U;
use txhdl::Trace;

#[derive(Trace, Default)]
pub struct Edges {
    pub count: Reg<U<4>>,
    pub captured: Reg<U<4>>,
    pub seen_four: Reg<bool>,
}

impl Edges {
    /// Counts on the rising edge.
    async fn count(&self) {
        loop {
            DefaultClock::rising().await;
            let n = self.count.get();
            self.count.set(n.wrapping_add(1));
        }
    }

    /// Captures on the falling edge, when the count the rising edge
    /// drove has been latched for half a cycle.
    async fn capture(&self) {
        loop {
            DefaultClock::falling().await;
            self.captured.set(self.count.get());
        }
    }

    /// A clocked condition on the falling edge.
    async fn watch(&self) {
        until(DefaultClock::falling, || self.count.get().raw() == 4).await;
        self.seen_four.set(true);
        println!("t={:>2} count reached four, seen at a falling edge", now());
    }
}

impl Module<(), ()> for Edges {
    async fn run(&mut self, _i: (), _o: ()) {
        join2(join2(self.count(), self.capture()), self.watch()).await;
    }
}

fn main() {
    let mut edges = Edges::default();
    if let Some(mut vcd) = Vcd::from_env() {
        vcd.clock::<DefaultClock>();
        vcd.add("edges", &edges);
        vcd.start();
    }
    let mut sim = Running::new(edges.run((), ()));
    for _ in 0..8 {
        sim.cycle();
    }
}
