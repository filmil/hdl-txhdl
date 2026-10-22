// SPDX-License-Identifier: Apache-2.0
//! A sending port that offers without waiting for room: `#[axi(port)]`.
//!
//! A `put` waits for room and only then raises `valid`, so its `valid`
//! waits on its `ready`. AXI forbids that of a source, and two netlists
//! joined wire to wire with it, the sender waiting on `ready` and the
//! receiver on `valid`, pass nothing (#1868). `#[axi(port)]` on `run`
//! marks a sending port whose `put` offers from the first edge it
//! waits at, and holds the word it read there until it is taken, as
//! AXI requires (#1871). A port it does not name is as before.
//!
//! `Offers` has two such ports. Its first process is a loop of one
//! wait, a `put` on `a`; its second a sequence of three waits, two of
//! them puts on `b`. Every word mixes in `seed`, an input the test
//! bench changes every cycle, so a word held while its offer waits is
//! the one read at the first edge, not what `seed` says later. The
//! consumers stall at random, and the netlist is checked against the
//! run, stalls and all, under nvc and Verilator.
use txhdl::comp::trace::{stop, Wave};
use txhdl::comp::{
    chan, join2, now, signal, Clock, DefaultClock, In, Reg, Running, Tx, Unit,
};
use txhdl::types::U;
use txhdl::{lower, Trace};

// begin{unit}
/// Two counts, each offered on a port marked `#[axi]`.
#[derive(Trace, Default)]
pub struct Offers {
    /// How many words `a` has given.
    pub n: Reg<U<8>>,
    /// How many pairs `b` has given.
    pub m: Reg<U<8>>,
}

#[lower]
impl Unit<In<U<8>>, (Tx<U<8>>, Tx<U<8>>)> for Offers {
    /// `a` offers its count mixed with `seed`; `b` offers a pair, its
    /// count plus `seed` and then the count moved on, and rests a
    /// cycle. Each offer is held until it is taken.
    #[axi(a, b)]
    async fn run(&mut self, seed: In<U<8>>, (a, b): (Tx<U<8>>, Tx<U<8>>)) {
        join2(
            async {
                loop {
                    a.put(|| self.n.get() ^ seed.get()).await;
                    self.n.set(self.n + 1);
                }
            },
            async {
                loop {
                    b.put(|| self.m.get() + seed.get()).await;
                    self.m.set(self.m + 1);
                    b.put(|| self.m.get()).await;
                    DefaultClock::rising().await;
                }
            },
        )
        .await;
    }
}
// end{unit}

fn main() {
    let (seed_out, seed) = signal::<U<8>, DefaultClock>();
    let (a, a_rx) = chan::<U<8>, DefaultClock>();
    let (b, b_rx) = chan::<U<8>, DefaultClock>();
    let mut offers = Offers::default();
    if let Some(mut w) = Wave::from_env() {
        w.clock::<DefaultClock>();
        // The ports are traced under the names `run` gives them.
        w.add("seed", &seed);
        w.add("offers", &offers);
        w.add("a", &a);
        w.add("b", &b);
        w.start();
    }
    let mut sim = Running::new(offers.run(seed, (a, b)));
    // `seed` changes every cycle, and each consumer takes when a bit of
    // a small shift register says so, about half the cycles.
    let mut lfsr: u8 = 0x5a;
    let (mut got_a, mut got_b): (Vec<u8>, Vec<u8>) = (Vec::new(), Vec::new());
    println!(" t seed    a    b");
    for _ in 0..60u32 {
        let bit = ((lfsr >> 7) ^ (lfsr >> 5) ^ (lfsr >> 4) ^ (lfsr >> 3)) & 1;
        lfsr = (lfsr << 1) | bit;
        seed_out.set(U::<8>::from(lfsr));
        let take = |rx: &txhdl::comp::Rx<U<8>>, go: bool, got: &mut Vec<u8>| {
            go.then(|| rx.recv())
                .flatten()
                .map(|v| {
                    got.push(v.raw() as u8);
                    format!("{:>4}", v.raw())
                })
                .unwrap_or_else(|| "   -".to_string())
        };
        let ta = take(&a_rx, lfsr & 1 == 1, &mut got_a);
        let tb = take(&b_rx, lfsr & 4 == 4, &mut got_b);
        sim.cycle();
        println!("{:2} {:>4} {ta} {tb}", now(), lfsr);
    }
    // The second word of each pair on `b` is the count moved on, which
    // no input touches: 1, 2, 3, and so on.
    let seconds: Vec<u8> = got_b.iter().skip(1).step_by(2).copied().collect();
    let counts: Vec<u8> = (1..=seconds.len() as u8).collect();
    assert_eq!(seconds, counts, "b's second words count up");
    assert!(got_a.len() >= 20 && got_b.len() >= 10, "words passed");
    stop();
    let net = Offers::lowered("offers");
    txhdl::netlist::write_vhdl_from_env(&net);
    print!("\n{}", net.verilog());
}
