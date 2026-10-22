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
//! consumers stall at random. The reset is held for the first three
//! cycles, and again for two in the middle, while `a`'s offer is held
//! up and `b` waits at its first `put`: no `valid` rises under it, and
//! after it each port reads its word again. The netlist is checked
//! against the run, stalls and resets and all, under nvc and Verilator.
use txhdl::comp::trace::{stop, Wave};
use txhdl::comp::{
    chan, join2, now, set_reset, signal, Clock, DefaultClock, In, Reg, Running,
    Tx, Unit,
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
    // a small shift register says so, about half the cycles. The reset
    // is held for the first three cycles; and from cycle 28 `a`'s
    // consumer stops, and two cycles of reset come at the first chance
    // after cycle 30 at which `b` waits at its first `put`: after the
    // second word of a pair, and the cycle it rests. A reset in the
    // middle of `b`'s sequence would part the run from the netlist,
    // whose `at_wait` goes back to the first wait (#1883).
    let mut lfsr: u8 = 0x5a;
    let (mut got_a, mut got_b): (Vec<u8>, Vec<u8>) = (Vec::new(), Vec::new());
    let (mut again, mut since_b): (Option<u32>, u32) = (None, 0);
    let mut pairs_before = 0;
    println!(" t seed    a    b");
    for i in 0..60u32 {
        let bit = ((lfsr >> 7) ^ (lfsr >> 5) ^ (lfsr >> 4) ^ (lfsr >> 3)) & 1;
        lfsr = (lfsr << 1) | bit;
        seed_out.set(U::<8>::from(lfsr));
        if again.is_none() && i >= 30 && got_b.len() % 2 == 0 && since_b >= 2 {
            again = Some(i);
            pairs_before = got_b.len() / 2;
        }
        let reset = i < 3 || again.is_some_and(|r| i < r + 2);
        set_reset(reset);
        let a_stops = (28..again.map_or(60, |r| r + 2)).contains(&i);
        let take = |rx: &txhdl::comp::Rx<U<8>>, go: bool, got: &mut Vec<u8>| {
            go.then(|| rx.recv())
                .flatten()
                .map(|v| {
                    got.push(v.raw() as u8);
                    format!("{:>4}", v.raw())
                })
                .unwrap_or_else(|| "   -".to_string())
        };
        let ta = take(&a_rx, !reset && !a_stops && lfsr & 1 == 1, &mut got_a);
        let tb = take(&b_rx, !reset && lfsr & 4 == 4, &mut got_b);
        since_b = if tb.trim() == "-" { since_b + 1 } else { 0 };
        sim.cycle();
        let r = if reset { " reset" } else { "" };
        println!("{:2} {:>4} {ta} {tb}{r}", now(), lfsr);
    }
    set_reset(false);
    // The second word of each pair on `b` is the count moved on, which
    // no input touches: 1, 2, 3, and so on, from 1 again after the
    // reset in the middle.
    let seconds: Vec<u8> = got_b.iter().skip(1).step_by(2).copied().collect();
    let counts: Vec<u8> = (1..=pairs_before as u8)
        .chain(1..=(seconds.len() - pairs_before) as u8)
        .collect();
    assert!(again.is_some(), "the reset in the middle came");
    assert_eq!(seconds, counts, "b's second words count up");
    assert!(got_a.len() >= 15 && got_b.len() >= 10, "words passed");
    stop();
    let net = Offers::lowered("offers");
    txhdl::netlist::write_vhdl_from_env(&net);
    print!("\n{}", net.verilog());
}
