// SPDX-License-Identifier: Apache-2.0
//! Two events at one edge. An adder needs a transaction from each of
//! two channels, so it waits for both inside `parallel!`, which
//! completes when both have arrived, and sums them. Producer A offers
//! every cycle and producer B every other one: A's word is taken when
//! it arrives and held in the group until B's does, A's channel holds
//! the next offer meanwhile, and so A runs at B's rate without a line
//! of code for it.
use txhdl::comp::trace::{stop, Wave};
use txhdl::comp::{chan, join2, now, Clock, DefaultClock, Reg, Rx, Tx, Unit};
use txhdl::parallel;
use txhdl::types::U;
use txhdl::Trace;

/// Offers a counting sequence every `period` cycles, when the channel
/// can take it.
pub struct Producer {
    pub name: &'static str,
    pub period: u8,
    pub seq: Reg<U<8>>,
    pub gap: Reg<U<8>>,
}

impl Producer {
    pub fn new(name: &'static str, period: u8) -> Self {
        Producer {
            name,
            period,
            seq: Reg::default(),
            gap: Reg::default(),
        }
    }
}

impl Unit<(), Tx<U<8>>> for Producer {
    async fn run(&mut self, _i: (), out: Tx<U<8>>) {
        loop {
            DefaultClock::rising().await;
            let gap = self.gap.get();
            let due = gap.raw() == 0 && out.ready().to_bool();
            self.gap.set((gap.raw() as u8 + 1) % self.period);
            if due {
                let seq = self.seq.get();
                out.send(seq);
                self.seq.set(seq + 1);
                println!("t={:>2} {} offers {}", now(), self.name, seq.raw());
            }
        }
    }
}

/// Waits for a word on each input, then sends their sum.
#[derive(Trace, Default)]
pub struct Adder {
    pub sums: Reg<U<8>>,
}

impl Unit<(Rx<U<8>>, Rx<U<8>>), Tx<U<8>>> for Adder {
    async fn run(&mut self, (a, b): (Rx<U<8>>, Rx<U<8>>), out: Tx<U<8>>) {
        loop {
            let (x, y) = parallel!(a.wait(), b.wait()).await;
            self.sums.set(self.sums + 1);
            out.send(x + y);
            println!("t={:>2} adder {} + {}", now(), x.raw(), y.raw());
        }
    }
}

/// Takes every sum and prints it.
#[derive(Default)]
pub struct Consumer {
    pub seen: Reg<U<8>>,
}

impl Unit<Rx<U<8>>, ()> for Consumer {
    async fn run(&mut self, inp: Rx<U<8>>, _o: ()) {
        loop {
            let s = inp.wait().await;
            self.seen.set(self.seen + 1);
            println!("t={:>2} sum {}", now(), s.raw());
        }
    }
}

fn main() {
    let (a_tx, a_rx) = chan::<U<8>, _>();
    let (b_tx, b_rx) = chan::<U<8>, _>();
    let (s_tx, s_rx) = chan::<U<8>, _>();
    let mut pa = Producer::new("A", 1);
    let mut pb = Producer::new("B", 2);
    let mut adder = Adder::default();
    let mut cons = Consumer::default();
    if let Some(mut vcd) = Wave::from_env() {
        vcd.clock::<DefaultClock>();
        vcd.add("a", &a_rx);
        vcd.add("b", &b_rx);
        vcd.add("sum", &s_rx);
        vcd.add("adder", &adder);
        vcd.start();
    }
    let mut sim = txhdl::comp::Running::new(join2(
        join2(pa.run((), a_tx), pb.run((), b_tx)),
        join2(adder.run((a_rx, b_rx), s_tx), cons.run(s_rx, ())),
    ));
    for _ in 0..8 {
        sim.cycle();
    }
    stop();
}
