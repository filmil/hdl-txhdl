// SPDX-License-Identifier: Apache-2.0
//! Whether a receive took something is a truth value.
//!
//! `recv_if(c)` takes a transaction when `c` holds and one is offered,
//! and gives it back as `Some`; asked `is_some()`, its answer is that
//! take, one bit, the condition and the channel's valid. A counter
//! offers its counts on a channel whose transaction is a word and a
//! flag, and a taker takes a word on every other cycle, shows whether
//! it took one, and counts what it took. The lowering once gave the
//! answer as the transaction's data, nine bits wide (issue 1079), which
//! nvc refused and Verilator ran: the one-bit port showed the data's
//! low bit, and the count followed the data rather than the takes.
use txhdl::comp::trace::{stop, Wave};
use txhdl::comp::{
    chan, join2, now, signal, Clock, DefaultClock, In, Out, Reg, Running, Rx,
    Tx, Unit,
};
use txhdl::types::{Bit, U};
use txhdl::{lower, Trace, Transaction as TransactionDerive, Value};

// begin{unit}
/// What the channels carry: a word and a flag, so that the data is
/// wider than the one bit a take is.
#[derive(TransactionDerive, Value, Clone, Copy, Default)]
pub struct Word {
    pub data: U<8>,
    pub odd: Bit,
}

/// Offers its next count whenever the channel has room.
#[derive(Trace, Default)]
pub struct Source {
    /// The next count to offer.
    pub n: Reg<U<8>>,
}

#[lower]
impl Unit for Source {
    async fn run(&mut self, _i: (), out: Tx<Word>) {
        loop {
            DefaultClock::rising().await;
            if out.ready().to_bool() {
                out.send(Word {
                    data: self.n.get(),
                    odd: self.n.get().bit(0),
                });
                self.n.set(self.n.get() + 1);
            }
        }
    }
}

/// Takes a word when told to, shows whether it took one, and counts
/// the takes.
#[derive(Trace, Default)]
pub struct Taker {
    pub taken: Reg<U<8>>,
}

#[lower]
impl Unit for Taker {
    async fn run(
        &mut self,
        (inp, go): (Rx<Word>, In<Bit>),
        (took, count): (Out<Bit>, Out<U<8>>),
    ) {
        loop {
            DefaultClock::rising().await;
            let got = inp.recv_if(go.get());
            let take = Bit::from(got.is_some());
            if take.to_bool() {
                self.taken.set(self.taken.get() + 1);
            }
            took.set(take);
            count.set(self.taken.get());
        }
    }
}

/// The two, and the channel between them.
#[derive(Trace, Default)]
pub struct Takers {
    pub source: Source,
    pub taker: Taker,
}

#[lower]
impl Unit for Takers {
    async fn run(&mut self, go: In<Bit>, (took, count): (Out<Bit>, Out<U<8>>)) {
        let (tx, rx) = chan::<Word, DefaultClock>();
        join2(
            self.source.run((), tx),
            self.taker.run((rx, go), (took, count)),
        )
        .await;
    }
}
// end{unit}

fn main() {
    let (go_out, go) = signal::<Bit, DefaultClock>();
    let (took_out, took) = signal::<Bit, DefaultClock>();
    let (count_out, count) = signal::<U<8>, DefaultClock>();
    let mut takers = Takers::default();
    if let Some(mut w) = Wave::from_env() {
        w.clock::<DefaultClock>();
        w.add("go", &go);
        w.add("took", &took);
        w.add("count", &count);
        w.add("takers", &takers);
        w.start();
    }
    let mut sim = Running::new(takers.run(go, (took_out, count_out)));

    // Two cycles for the channel to fill, then a take on every other
    // cycle: eight takes in sixteen.
    go_out.set(Bit::Zero);
    sim.cycle();
    sim.cycle();
    for k in 0..16 {
        go_out.set(Bit::from(k % 2 == 0));
        sim.cycle();
    }
    sim.cycle();
    println!("t={:>2} took {} words", now(), count.get().raw());
    stop();
    assert_eq!(count.get().raw(), 8, "a take is one, and eight were taken");
    let net = Takers::lowered("takers");
    txhdl::netlist::write_vhdl_from_env(&net);
    print!("\n{}", net.verilog());
}
