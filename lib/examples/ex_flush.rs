// SPDX-License-Identifier: Apache-2.0
//! A reset empties the channel between two units.
//!
//! The pair below is a counter that offers each count while its
//! channel has room, and a taker that takes one only while it is told
//! to. Told nothing, the taker leaves the channel full, with the
//! first two counts in it; then the reset comes, and after it the
//! taker is told to take. What it takes starts at the counter's first
//! count again, with no stale words from before the reset: a channel
//! is two words and two valid bits, the reset clears the bits, and a
//! channel with neither bit set is empty (issue 729). The run and the
//! netlist are checked against each other under nvc and Verilator
//! through the reset and out the other side.
use txhdl::comp::trace::{stop, Wave};
use txhdl::comp::{
    chan, join2, now, set_reset, signal, Clock, DefaultClock, In, Out, Reg,
    Running, Rx, Tx, Unit,
};
use txhdl::types::{Bit, U};
use txhdl::{lower, Trace};

// begin{unit}
/// Offers the next count whenever the channel has room.
#[derive(Trace, Default)]
pub struct Counter {
    /// The next count to offer.
    pub n: Reg<U<8>>,
}

#[lower]
impl Unit for Counter {
    async fn run(&mut self, _i: (), out: Tx<U<8>>) {
        loop {
            DefaultClock::rising().await;
            if out.ready().to_bool() {
                out.send(self.n);
                self.n.set(self.n + 1);
            }
        }
    }
}

/// Takes a word while `take` is high, and shows the last it took.
#[derive(Trace, Default)]
pub struct Taker {
    /// The last word taken.
    pub last: Reg<U<8>>,
}

#[lower]
impl Unit for Taker {
    async fn run(
        &mut self,
        (inp, take): (Rx<U<8>>, In<Bit>),
        shown: Out<U<8>>,
    ) {
        loop {
            DefaultClock::rising().await;
            let go = take.get() & inp.peek().is_some();
            let w = inp.head();
            let _ = inp.recv_if(go);
            if go.to_bool() {
                self.last.set(w);
            }
            shown.set(self.last);
        }
    }
}

/// The two, and the channel between them.
#[derive(Trace, Default)]
pub struct Pair {
    pub counter: Counter,
    pub taker: Taker,
}

#[lower]
impl Unit for Pair {
    async fn run(&mut self, take: In<Bit>, shown: Out<U<8>>) {
        let (tx, rx) = chan::<U<8>, DefaultClock>();
        join2(self.counter.run((), tx), self.taker.run((rx, take), shown))
            .await;
    }
}
// end{unit}

fn main() {
    let (take_out, take) = signal::<Bit, DefaultClock>();
    let (shown_out, shown) = signal::<U<8>, DefaultClock>();
    let mut pair = Pair::default();
    if let Some(mut w) = Wave::from_env() {
        w.clock::<DefaultClock>();
        w.add("take", &take);
        w.add("shown", &shown);
        w.add("pair", &pair);
        w.start();
    }
    let mut sim = Running::new(pair.run(take, shown_out));

    // Nothing taken: the channel fills with the first two counts.
    take_out.set(Bit::Zero);
    for _ in 0..4 {
        sim.cycle();
    }
    println!("t={:>2} channel full, nothing taken", now());

    // The reset, for two cycles, with nothing taken.
    set_reset(true);
    for _ in 0..2 {
        sim.cycle();
    }
    set_reset(false);
    println!("t={:>2} reset released", now());

    // Take a word a cycle. The first is the counter's first count, and
    // no word from before the reset comes out.
    take_out.set(Bit::One);
    let mut seen = Vec::new();
    for _ in 0..8 {
        sim.cycle();
        seen.push(shown.get().raw() as u8);
        println!("t={:>2} shown {}", now(), shown.get().raw());
    }
    stop();
    // What the taker shows starts at its own zero, then the counter's
    // first count, zero again, and climbs from there; a word from
    // before the reset would have shown as a 1 and then a 0.
    assert_eq!(seen, [0, 0, 0, 1, 2, 3, 4, 5], "no stale word after reset");
    let net = Pair::lowered("pair");
    txhdl::netlist::write_vhdl_from_env(&net);
    print!("\n{}", net.verilog());
}
