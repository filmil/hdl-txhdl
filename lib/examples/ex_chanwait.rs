// SPDX-License-Identifier: Apache-2.0
//! A loop whose only wait is a channel's (issue 881).
//!
//! `tx.put(|| v).await` waits for an edge of the channel's clock at
//! which it has room, and `rx.wait().await` for one at which it holds a
//! word: each is a wait for an edge, the channel's own, and a loop with
//! nothing else in it needs no `C::rising()` before it. The lowering
//! takes the loop's clock from the channel's port, as the run takes the
//! edge from the channel's type.
//!
//! `Count` offers 0, 1, 2 and on, a word each time the last is taken,
//! and its only wait is the `put`. `Last` shows the last word it took,
//! and its only wait is the `wait`. `Pair` joins the two by a channel,
//! and the build simulates the netlist against the run under nvc and
//! Verilator.
use txhdl::comp::trace::{stop, Wave};
use txhdl::comp::{
    chan, join2, now, signal, DefaultClock, Out, Reg, Running, Rx, Tx, Unit,
};
use txhdl::types::U;
use txhdl::{lower, Trace};

// begin{unit}
/// Offers its count, and counts on once it is taken.
#[derive(Trace, Default)]
pub struct Count {
    pub n: Reg<U<8>>,
}

#[lower]
impl Unit<(), Tx<U<8>>> for Count {
    async fn run(&mut self, _i: (), out: Tx<U<8>>) {
        loop {
            out.put(|| self.n.get()).await;
            self.n.set(self.n + 1);
        }
    }
}

/// Shows the last word it took.
#[derive(Trace, Default)]
pub struct Last {}

#[lower]
impl Unit<Rx<U<8>>, Out<U<8>>> for Last {
    async fn run(&mut self, inp: Rx<U<8>>, last: Out<U<8>>) {
        loop {
            let v = inp.wait().await;
            last.set(v);
        }
    }
}

/// The two, by a channel.
#[derive(Trace, Default)]
pub struct Pair {
    pub count: Count,
    pub last: Last,
}

#[lower]
impl Unit<(), Out<U<8>>> for Pair {
    async fn run(&mut self, _i: (), shown: Out<U<8>>) {
        let (tx, rx) = chan::<U<8>, DefaultClock>();
        join2(self.count.run((), tx), self.last.run(rx, shown)).await;
    }
}
// end{unit}

fn main() {
    let (shown_out, shown) = signal::<U<8>, DefaultClock>();
    let mut pair = Pair::default();
    if let Some(mut wave) = Wave::from_env() {
        wave.clock::<DefaultClock>();
        wave.add("shown", &shown);
        wave.add("pair", &pair);
        wave.start();
    }
    let mut sim = Running::new(pair.run((), shown_out));
    let mut seen = Vec::new();
    for _ in 0..12 {
        sim.cycle();
        seen.push(shown.get().raw() as u8);
    }
    println!("t={:>2} shown {:?}", now(), seen);
    // Every word arrives, in order, from zero.
    let mut taken: Vec<u8> = seen.clone();
    taken.dedup();
    assert_eq!(taken[0], 0, "from zero");
    assert!(taken.windows(2).all(|w| w[1] == w[0] + 1), "in order");
    assert!(*taken.last().unwrap() >= 4, "and on");

    let net = Pair::lowered("chanpair");
    print!("\n{}", net.verilog());
    stop();
    txhdl::netlist::write_vhdl_from_env(&net);
}
