// SPDX-License-Identifier: Apache-2.0
//! Running one thing twice and comparing: a stream sent to two copies
//! of a unit, and their answers taken back and checked.
//!
//! `Tee` hands each word to both copies in one cycle, or to neither,
//! so neither copy can run ahead. `Check` takes a word from each, in
//! the same cycle, passes the first copy's on, and raises a line when
//! the two differ; the line stays up until it is reset.
//!
//! The copies here are two adders, and one of them is given a
//! different addend halfway through, which is what a fault looks like
//! from the outside: the same input, a different answer. The run
//! prints what leaves the check and what the line says, and the build
//! simulates the netlists of the tee and of the check against this run
//! under nvc and Verilator.
use txhdl::comp::trace::{stop, Wave};
use txhdl::comp::{
    chan, join2, now, signal, Clock, DefaultClock, In, Reg, Running, Rx, Tx,
    Unit,
};
use txhdl::types::{Bit, U};
use txhdl::{lower, with, Trace, Transaction, Value};
use txhdl_parts::redundant::{Check, Tee};

// begin{adder}
/// One word in, that word plus `bump`, out. Two of these are the
/// copies, and `bump` is what a fault changes.
#[derive(Trace, Default)]
pub struct Adder {
    /// The last word it answered, which the netlist keeps.
    pub last: Reg<U<8>>,
}

#[lower]
impl Unit for Adder {
    async fn run(
        &mut self,
        (bump, inp): (In<U<8>>, Rx<Word>),
        out: Tx<Word>,
    ) {
        loop {
            DefaultClock::rising().await;
            let go = inp.peek().is_some() & out.ready();
            let word = inp.head();
            let _ = inp.recv_if(go);
            let sum = word.v + bump.get();
            with!(self <= { go ? last: sum });
            if go.to_bool() {
                out.send(Word { v: sum });
            }
        }
    }
}
// end{adder}

/// What the channels carry: one byte.
#[derive(Transaction, Value, Clone, Copy, Default, Debug, PartialEq)]
pub struct Word {
    /// The byte.
    pub v: U<8>,
}

fn main() {
    let (src_tx, src_rx) = chan::<Word, DefaultClock>();
    let (a_tx, a_rx) = chan::<Word, DefaultClock>();
    let (b_tx, b_rx) = chan::<Word, DefaultClock>();
    let (ra_tx, ra_rx) = chan::<Word, DefaultClock>();
    let (rb_tx, rb_rx) = chan::<Word, DefaultClock>();
    let (out_tx, out_rx) = chan::<Word, DefaultClock>();
    let (rst_out, rst) = signal::<Bit, DefaultClock>();
    let (bump1_out, bump1) = signal::<U<8>, DefaultClock>();
    let (bump2_out, bump2) = signal::<U<8>, DefaultClock>();
    let (differs_out, differs) = signal::<Bit, DefaultClock>();
    let mut tee = Tee::<Word>::default();
    let mut one = Adder::default();
    let mut two = Adder::default();
    let mut check = Check::<Word>::default();

    if let Some(mut w) = Wave::from_env() {
        w.clock::<DefaultClock>();
        w.add("src", &src_rx);
        w.add("a", &a_rx);
        w.add("b", &b_rx);
        w.add("ra", &ra_rx);
        w.add("rb", &rb_rx);
        w.add("out", &out_rx);
        w.add("rst", &rst);
        w.add("bump2", &bump2);
        w.add("differs", &differs);
        w.add("tee", &tee);
        w.add("check", &check);
        w.start();
    }

    let client = async move {
        let mut v = 1u8;
        while v <= 6 {
            DefaultClock::rising().await;
            // A word only when the tee has room for one, since a
            // channel holds two and the tee waits for both copies.
            if src_tx.ready().to_bool() {
                src_tx.send(Word { v: U::from(v) });
                v += 1;
            }
        }
    };
    let sink = async move {
        loop {
            DefaultClock::rising().await;
            if let Some(w) = out_rx.recv() {
                println!(
                    "t={:>2} out {:>3}  differs {}",
                    now(),
                    w.v.raw(),
                    differs.get().to_bool() as u8
                );
            }
        }
    };
    // Downstream first: a unit reads the readiness of the one after
    // it in the same cycle, so the one after it has to have settled.
    let mut sim = Running::new(join2(
        join2(
            sink,
            check.run((rst, ra_rx, rb_rx), (out_tx, differs_out)),
        ),
        join2(
            join2(
                one.run((bump1, a_rx), ra_tx),
                two.run((bump2, b_rx), rb_tx),
            ),
            join2(tee.run(src_rx, (a_tx, b_tx)), client),
        ),
    ));
    println!("  t  what left the check");
    rst_out.set(Bit::One);
    bump1_out.set(U::from(1u8));
    bump2_out.set(U::from(1u8));
    sim.cycle();
    rst_out.set(Bit::Zero);
    for c in 0..18 {
        // The fault: from the fourth cycle the second copy adds two,
        // so the first three words agree and the rest do not.
        bump2_out.set(U::from(if c >= 4 { 2u8 } else { 1u8 }));
        sim.cycle();
    }
    stop();
    txhdl::netlist::write_netlists_from_env(&[
        &Tee::<Word>::lowered("tee"),
        &Check::<Word>::lowered("check"),
    ]);
    print!("\n{}", Tee::<Word>::verilog("tee"));
}
