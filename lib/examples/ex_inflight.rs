// SPDX-License-Identifier: Apache-2.0
//! Several invocations in flight. `mac` is an async fn with two awaits,
//! a multiply and an add, so it takes two cycles, and `pipeline::drive`
//! starts one invocation per cycle: at any edge one is in its
//! multiplier and another in its adder, and a result lands every cycle
//! from the third on. Nobody placed a stage boundary; the two awaits
//! are the boundaries, and each invocation prints where it is.
use txhdl::comp::trace::{stop, Wave};
use txhdl::comp::{chan, join2, now, Clock, DefaultClock, Reg, Rx, Tx, Unit};
use txhdl::pipeline::{add, drive, mul};
use txhdl::types::U;
use txhdl::Transaction;
use txhdl::Value;

#[derive(Transaction, Value, Clone, Copy, Default)]
pub struct Pair {
    pub a: U<8>,
    pub b: U<8>,
}

/// The pipeline: `a * b + 1`, in two cycles. Written as any async fn;
/// `drive` is what makes it a pipeline rather than a sequence.
async fn mac(p: Pair) -> U<64> {
    println!("t={:>2}   [{}] in the multiplier", now(), p.a.raw());
    let (a, b) = (
        U::<32>::from(p.a.raw() as u32),
        U::<32>::from(p.b.raw() as u32),
    );
    let prod = mul(a, b).await;
    println!("t={:>2}   [{}] in the adder", now(), p.a.raw());
    add(prod, U::<64>::from(1)).await
}

/// Offers a pair every cycle.
#[derive(Default)]
pub struct Source {
    pub n: Reg<U<8>>,
}

impl Unit<(), Tx<Pair>> for Source {
    async fn run(&mut self, _i: (), out: Tx<Pair>) {
        loop {
            DefaultClock::rising().await;
            let n = self.n.get();
            if out.ready().to_bool() {
                out.send(Pair { a: n, b: n });
                self.n.set(n + 1);
                println!("t={:>2} start [{}]", now(), n.raw());
            }
        }
    }
}

/// Takes every result.
#[derive(Default)]
pub struct Sink {
    pub seen: Reg<U<32>>,
}

impl Unit<Rx<U<64>>, ()> for Sink {
    async fn run(&mut self, inp: Rx<U<64>>, _o: ()) {
        loop {
            let v = inp.wait().await;
            let seen = self.seen.get();
            self.seen.set(seen + 1);
            println!("t={:>2} done  {}", now(), v.raw());
        }
    }
}

fn main() {
    let (p_tx, p_rx) = chan::<Pair, _>();
    let (r_tx, r_rx) = chan::<U<64>, _>();
    let mut source = Source::default();
    let mut sink = Sink::default();
    if let Some(mut vcd) = Wave::from_env() {
        vcd.clock::<DefaultClock>();
        vcd.add("pair", &p_rx);
        vcd.add("result", &r_rx);
        vcd.start();
    }
    let mut sim = txhdl::comp::Running::new(join2(
        join2(source.run((), p_tx), drive(mac, p_rx, r_tx)),
        sink.run(r_rx, ()),
    ));
    for _ in 0..6 {
        sim.cycle();
    }
    stop();
}
