// SPDX-License-Identifier: Apache-2.0
//! An asynchronous FIFO. The producer port is in one clock and the
//! consumer port in another, and the two clocks stand in a stated
//! relation: `ClkW` has period 2, `ClkR` period 3 and phase 1, in the
//! unit the design shares. Each pointer lives in its own domain and
//! crosses to the other through a `Crossing`, which is what makes the
//! memory read on the consumer side safe; the memory itself is written
//! by the producer clock and read without a wait, as a dual-port RAM is.
use txhdl::comp::{
    chan, join2, now, Clock, Crossing, In, Mem, Module, Out, Reg, Running, Rx,
    Tx,
};
use txhdl::types::{Bit, U};
use txhdl::when;

pub struct ClkW;
impl Clock for ClkW {
    const NAME: &'static str = "clk_w";
    const PERIOD: u64 = 2;
}
pub struct ClkR;
impl Clock for ClkR {
    const NAME: &'static str = "clk_r";
    const PERIOD: u64 = 3;
    const PHASE: u64 = 1;
}

/// A pointer that lives in `A` and is read in `B`. The `Out` is driven
/// by the pointer's own process, the `Crossing` is stepped by the
/// reading process at its own edge, which is what a synchroniser does,
/// and the `In` is what that process reads.
pub struct Synced<A: Clock, B: Clock> {
    out: Out<U<8>, A>,
    x: Crossing<U<8>, A, B>,
    inp: In<U<8>, B>,
}

impl<A: Clock, B: Clock> Default for Synced<A, B> {
    fn default() -> Self {
        let (out, a) = txhdl::comp::signal::<U<8>, A>();
        let (x, inp) = Crossing::new(a);
        Synced { out, x, inp }
    }
}

/// `N` words, written in `W` and read in `R`. The pointers are one bit
/// wider than the address so that full and empty tell apart.
pub struct Fifo<const N: usize, W: Clock, R: Clock> {
    mem: Mem<U<8>, N, W>,
    wptr: Reg<U<8>, W>,
    rptr: Reg<U<8>, R>,
    wptr_r: Synced<W, R>,
    rptr_w: Synced<R, W>,
}

/// Written out, because a derived `Default` would ask the clocks to be
/// `Default` too, and a clock is a type with nothing in it.
impl<const N: usize, W: Clock, R: Clock> Default for Fifo<N, W, R> {
    fn default() -> Self {
        Fifo {
            mem: Mem::default(),
            wptr: Reg::default(),
            rptr: Reg::default(),
            wptr_r: Synced::default(),
            rptr_w: Synced::default(),
        }
    }
}

impl<const N: usize, W: Clock, R: Clock> Fifo<N, W, R> {
    async fn write(&self, push: Rx<U<8>, W>) {
        loop {
            let w = self.wptr.get().await;
            self.rptr_w.x.step();
            let r = self.rptr_w.inp.get();
            let full = Bit::from_bool(w.wrapping_sub(r) == U::from(N as u8));
            let offer = push.peek();
            let take = full.not().and(Bit::from_bool(offer.is_some()));
            when!(take => { self.wptr <= w.wrapping_add(1) });
            if take.to_bool() {
                self.mem.write(w.raw() as usize, offer.unwrap_or_default());
                push.recv();
            }
            self.wptr_r.out.set(w);
        }
    }

    async fn read(&self, pop: Tx<U<8>, R>) {
        loop {
            let r = self.rptr.get().await;
            self.wptr_r.x.step();
            let w = self.wptr_r.inp.get();
            let empty = Bit::from_bool(r == w);
            let give = empty.not().and(pop.ready());
            when!(give => { self.rptr <= r.wrapping_add(1) });
            if give.to_bool() {
                pop.send(self.mem.read(r.raw() as usize))
            }
            self.rptr_w.out.set(r);
        }
    }
}

impl<const N: usize, W: Clock, R: Clock> Module<Rx<U<8>, W>, Tx<U<8>, R>>
    for Fifo<N, W, R>
{
    async fn run(&mut self, push: Rx<U<8>, W>, pop: Tx<U<8>, R>) {
        join2(self.write(push), self.read(pop)).await;
    }
}

/// Offers a counting sequence whenever the channel can take one.
#[derive(Default)]
pub struct Producer {
    n: Reg<U<8>, ClkW>,
}

impl Module<(), Tx<U<8>, ClkW>> for Producer {
    async fn run(&mut self, _i: (), push: Tx<U<8>, ClkW>) {
        loop {
            let n = self.n.get().await;
            let ready = push.ready();
            when!(ready => { self.n <= n.wrapping_add(1) });
            if ready.to_bool() {
                push.send(n);
                println!("t={:>2} {}: push {}", now(), ClkW::NAME, n.raw())
            } else {
                println!("t={:>2} {}: full", now(), ClkW::NAME)
            }
        }
    }
}

/// Takes whatever arrives.
#[derive(Default)]
pub struct Consumer {
    seen: Reg<U<8>, ClkR>,
}

impl Module<Rx<U<8>, ClkR>, ()> for Consumer {
    async fn run(&mut self, pop: Rx<U<8>, ClkR>, _o: ()) {
        loop {
            let seen = self.seen.get().await;
            match pop.recv() {
                Some(v) => {
                    self.seen.set(seen.wrapping_add(1));
                    println!("t={:>2} {}: pop  {}", now(), ClkR::NAME, v.raw())
                }
                None => println!("t={:>2} {}: empty", now(), ClkR::NAME),
            }
        }
    }
}

fn main() {
    let (push_tx, push_rx) = chan::<U<8>, ClkW>();
    let (pop_tx, pop_rx) = chan::<U<8>, ClkR>();
    let mut producer = Producer::default();
    let mut fifo = Fifo::<4, ClkW, ClkR>::default();
    let mut consumer = Consumer::default();
    let mut sim = Running::new(join2(
        join2(producer.run((), push_tx), fifo.run(push_rx, pop_tx)),
        consumer.run(pop_rx, ()),
    ));
    for _ in 0..20 {
        sim.step();
    }
}
