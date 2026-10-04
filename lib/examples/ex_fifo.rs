// SPDX-License-Identifier: Apache-2.0
//! An asynchronous FIFO between two clocks. The producer is in one
//! clock and the consumer in another, and the two clocks stand in a
//! stated relation: `ClkW` has period 4, `ClkR` period 6 and phase 2, in
//! the unit the design shares. Between them is `ChanCdc` from the parts,
//! the one way a value crosses from one clock to another (issue 1017):
//! a memory of four words written on the producer's clock and read on
//! the consumer's, and each side's pointer Gray coded and carried to the
//! other through two flip-flops of the other's clock. Each side waits
//! for its own event, and the channel's own handshake is the
//! backpressure: the producer waits until the channel is ready, the
//! consumer for a word.
use txhdl::comp::{chan, join2, now, until, Clock, Reg, Running, Rx, Tx, Unit};
use txhdl::types::U;
use txhdl_parts::cdc::ChanCdc;

pub struct ClkW;
impl Clock for ClkW {
    const NAME: &'static str = "clk_w";
    const PERIOD: u64 = 4;
}
pub struct ClkR;
impl Clock for ClkR {
    const NAME: &'static str = "clk_r";
    const PERIOD: u64 = 6;
    const PHASE: u64 = 2;
}

/// Four words of `U<8>`, two address bits and three pointer bits.
pub type Fifo = ChanCdc<U<8>, 2, 4, 3, ClkW, ClkR>;

/// Offers a counting sequence whenever the channel can take one.
#[derive(Default)]
pub struct Producer {
    n: Reg<U<8>, ClkW>,
}

impl Unit<(), Tx<U<8>, ClkW>> for Producer {
    async fn run(&mut self, _i: (), push: Tx<U<8>, ClkW>) {
        loop {
            until(ClkW::rising, || push.ready().to_bool()).await;
            let n = self.n.get();
            push.send(n);
            self.n.set(n + 1);
            println!("t={:>2} {}: push {}", now(), ClkW::NAME, n.raw());
        }
    }
}

/// Takes whatever arrives.
#[derive(Default)]
pub struct Consumer {
    seen: Reg<U<8>, ClkR>,
}

impl Unit<Rx<U<8>, ClkR>, ()> for Consumer {
    async fn run(&mut self, pop: Rx<U<8>, ClkR>, _o: ()) {
        loop {
            let v = pop.wait().await;
            self.seen.set(self.seen + 1);
            println!("t={:>2} {}: pop  {}", now(), ClkR::NAME, v.raw());
        }
    }
}

fn main() {
    let (push_tx, push_rx) = chan::<U<8>, ClkW>();
    let (pop_tx, pop_rx) = chan::<U<8>, ClkR>();
    let mut producer = Producer::default();
    let mut fifo = Fifo::default();
    let mut consumer = Consumer::default();
    let mut sim = Running::new(join2(
        join2(producer.run((), push_tx), fifo.run(push_rx, pop_tx)),
        consumer.run(pop_rx, ()),
    ));
    for _ in 0..40 {
        sim.step();
    }
}
