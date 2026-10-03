// SPDX-License-Identifier: Apache-2.0
//! An offer that completes on the edge it is taken: `tx.put(|| v)`.
//!
//! A sequence that walks a store and offers each word needs its
//! address to move exactly on the edge the word is taken, and not an
//! edge later, or the next offer reads the word it has just given.
//! `put` is that: it waits for an edge at which the channel has room,
//! sends in the step that edge begins, and what follows it happens in
//! that step, so a register set after it takes its value on the take
//! (issue 755). The word is a closure, read at that edge as `until`
//! reads its condition, since a word read where `put` is called is
//! read before the address has moved.
//!
//! The walker takes eight words into its store, a word a cycle as they
//! come, and then puts them out in order: the loader is written a
//! cycle at a time, as a store's writes are, and the offerer is a
//! sequence. Its consumer here stalls at random, so an offer is held
//! for a cycle or several before it is taken; the run checks that
//! every word comes back once and in order, and the netlist is checked
//! against the run, stalls and all, under nvc and Verilator.
use txhdl::comp::trace::{stop, Wave};
use txhdl::comp::{
    chan, join2, now, until, Clock, DefaultClock, Mem, Reg, Running, Rx, Tx,
    Unit,
};
use txhdl::types::{Bit, U};
use txhdl::{lower, with, Trace};

// begin{unit}
/// Takes eight words into a store, then offers them in order, each
/// until it is taken.
#[derive(Trace, Default)]
pub struct Walker {
    /// The words, as they came.
    pub table: Mem<U<8>, 8>,
    /// Where the next word goes in.
    pub fill: Reg<U<3>>,
    /// The store holds eight words, and they are being offered.
    pub full: Reg<Bit>,
    /// Up for the one cycle after the last is taken, which lets the
    /// next eight in.
    pub done: Reg<Bit>,
    /// Where the next word comes out: the store's one read address.
    pub at: Reg<U<3>>,
}

#[lower]
impl Unit<Rx<U<8>>, Tx<U<8>>> for Walker {
    /// Two processes. The loader takes a word a cycle while the store
    /// is not full. The offerer is the sequence: wait for a full
    /// store, then put its words out one by one.
    async fn run(&mut self, words: Rx<U<8>>, out: Tx<U<8>>) {
        join2(
            async {
                loop {
                    DefaultClock::rising().await;
                    let fill = self.fill.get();
                    let loading = !self.full.get();
                    let take = loading & words.peek().is_some();
                    let w = words.head();
                    let _ = words.recv_if(loading);
                    with!(self <= {
                        take ? fill: fill + 1,
                        take & (fill == 7) ? full: Bit::One,
                        self.done.get() ? full: Bit::Zero,
                    });
                    if take.to_bool() {
                        self.table.at(fill).set(w);
                    }
                }
            },
            async {
                loop {
                    until(DefaultClock::rising, || self.full.get().to_bool())
                        .await;
                    // Each word put until it is taken, and the address
                    // moved on at the edge it is.
                    for _ in 0..8 {
                        out.put(|| self.table.read(self.at.get())).await;
                        self.at.set(self.at.get() + 1);
                    }
                    self.done.set(Bit::One);
                    DefaultClock::rising().await;
                    self.done.set(Bit::Zero);
                }
            },
        )
        .await;
    }
}
// end{unit}

fn main() {
    let (word_tx, words) = chan::<U<8>, DefaultClock>();
    let (out, out_rx) = chan::<U<8>, DefaultClock>();
    let mut walker = Walker::default();
    let at = walker.at;
    if let Some(mut w) = Wave::from_env() {
        w.clock::<DefaultClock>();
        // The ports are traced under the names `run` gives them.
        w.add("words", &words);
        w.add("walker", &walker);
        w.add("out", &out);
        w.start();
    }
    let mut sim = Running::new(walker.run(words, out));
    // Two rounds of eight words, sent as fast as the walker takes
    // them, and a consumer that takes when a small shift register says
    // so: about half the cycles, in runs of a cycle or several.
    let sent: Vec<u8> = (0..16u8)
        .map(|i| i.wrapping_mul(37).wrapping_add(11))
        .collect();
    let mut next = 0usize;
    let mut got: Vec<u8> = Vec::new();
    let mut lfsr: u8 = 0x5a;
    println!(" t word at take");
    for _ in 0..80u32 {
        let word = if next < sent.len() && word_tx.ready().to_bool() {
            word_tx.send(U::<8>::from(sent[next]));
            next += 1;
            format!("{:>4}", sent[next - 1])
        } else {
            "   -".to_string()
        };
        let bit = ((lfsr >> 7) ^ (lfsr >> 5) ^ (lfsr >> 4) ^ (lfsr >> 3)) & 1;
        lfsr = (lfsr << 1) | bit;
        let take = if lfsr & 1 == 1 {
            out_rx.recv().map(|v| {
                got.push(v.raw() as u8);
                format!("{:>4}", v.raw())
            })
        } else {
            None
        };
        sim.cycle();
        println!(
            "{:2} {word} {:>2} {}",
            now(),
            at.get().raw(),
            take.unwrap_or_else(|| "   -".to_string())
        );
    }
    assert_eq!(got, sent, "every word came back once, in order");
    stop();
    let net = Walker::lowered("walker");
    txhdl::netlist::write_vhdl_from_env(&net);
    print!("\n{}", net.verilog());
}
