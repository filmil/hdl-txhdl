// SPDX-License-Identifier: Apache-2.0
//! A round-robin arbiter: two producers, one consumer. The arbiter
//! waits until either input offers, then reads its turn register and
//! takes from the side whose turn it is if that side offered, else from
//! the other; the winner passes the turn. The turn is read after the
//! wait, at the edge, which is what makes it the turn at that edge: a
//! version that read it before the wait, to fix the order of a race,
//! saw the previous cycle's turn and granted A twice in a row. Producer
//! A offers every cycle, producer B every third one.
use txhdl::comp::{
    chan, join2, now, until, Clock, DefaultClock, Reg, Rx, Tx, Unit,
};
use txhdl::types::{Bit, U};
use txhdl::{when, Transaction};

#[derive(Transaction, Clone, Copy, Default, Debug)]
pub struct Packet {
    pub from: Bit,
    pub seq: U<8>,
}

/// Offers a numbered packet whenever the channel is ready and the gap
/// counter is at zero; the counter counts to `period` and wraps, so a
/// period of 1 offers every cycle and a period of 3 every third.
pub struct Producer {
    pub from: Bit,
    pub period: u8,
    pub seq: Reg<U<8>>,
    pub gap: Reg<U<8>>,
}

impl Producer {
    pub fn new(from: Bit, period: u8) -> Self {
        Producer {
            from,
            period,
            seq: Reg::default(),
            gap: Reg::default(),
        }
    }
}

impl Unit<(), Tx<Packet>> for Producer {
    async fn run(&mut self, _i: (), out: Tx<Packet>) {
        loop {
            DefaultClock::rising().await;
            let (seq, gap) = (self.seq.get(), self.gap.get());
            let due = gap == 0;
            let offer = out.ready() & due;
            let next_gap = (gap.raw() as u8 + 1) % self.period;
            when!(offer => { self.seq <= seq + 1 });
            self.gap.set(next_gap);
            if offer.to_bool() {
                out.send(Packet {
                    from: self.from,
                    seq,
                });
            }
        }
    }
}

/// Waits until either input offers. The turn, read at that edge, says
/// which side is served when both do, and the side served passes the
/// turn to the other.
#[derive(Default)]
pub struct Arbiter {
    pub turn: Reg<Bit>,
}

impl Unit<(Rx<Packet>, Rx<Packet>), Tx<Packet>> for Arbiter {
    async fn run(&mut self, (a, b): (Rx<Packet>, Rx<Packet>), out: Tx<Packet>) {
        loop {
            until(DefaultClock::rising, || {
                (a.peek().is_some() || b.peek().is_some())
                    && out.ready().to_bool()
            })
            .await;
            let turn = self.turn.get();
            let (first, second) =
                if turn.to_bool() { (&b, &a) } else { (&a, &b) };
            let p = match first.recv() {
                Some(p) => p,
                None => second.recv().unwrap_or_default(),
            };
            out.send(p);
            self.turn.set(!p.from);
            let name = if p.from.to_bool() { "B" } else { "A" };
            println!("t={:>2} grant {} {}", now(), name, p.seq.raw());
        }
    }
}

/// Takes whatever arrives.
#[derive(Default)]
pub struct Consumer {
    pub taken: Reg<U<8>>,
}

impl Unit<Rx<Packet>, ()> for Consumer {
    async fn run(&mut self, inp: Rx<Packet>, _o: ()) {
        loop {
            inp.wait().await;
            let taken = self.taken.get();
            self.taken.set(taken + 1);
        }
    }
}

fn main() {
    let (a_tx, a_rx) = chan::<Packet, _>();
    let (b_tx, b_rx) = chan::<Packet, _>();
    let (out_tx, out_rx) = chan::<Packet, _>();
    let mut pa = Producer::new(Bit::Zero, 1);
    let mut pb = Producer::new(Bit::One, 3);
    let mut arb = Arbiter::default();
    let mut cons = Consumer::default();
    let mut sim = txhdl::comp::Running::new(join2(
        join2(pa.run((), a_tx), pb.run((), b_tx)),
        join2(arb.run((a_rx, b_rx), out_tx), cons.run(out_rx, ())),
    ));
    for _ in 0..12 {
        sim.cycle();
    }
}
