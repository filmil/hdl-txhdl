// SPDX-License-Identifier: Apache-2.0
//! An asynchronous FIFO. Each side waits for its own event: the write
//! side for an offer it has room for, the read side for a word and a
//! consumer able to take it. The producer port is in one clock and the
//! consumer port in another, and the two clocks stand in a stated
//! relation: `ClkW` has period 4, `ClkR` period 6 and phase 2, in the
//! unit the design shares. Each pointer lives in its own domain and
//! crosses to the other through a `Crossing`, which is what makes the
//! memory read on the consumer side safe; the memory itself is written
//! by the producer clock and read without a wait, as a dual-port RAM is.
use txhdl::comp::{
    chan, join2, now, until, Clock, Crossing, In, Mem, Out, Reg, Running, Rx,
    Tx, Unit,
};
use txhdl::types::U;

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

/// A pointer that lives in `A` and is read in `B`. The `Out` is driven
/// by the pointer's own process, the `Crossing` runs as a process of
/// its own and samples at every edge of `B`, which is what a
/// synchroniser does, and the `In` is what the reading process reads.
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
    fn full(&self) -> bool {
        self.wptr.get() - self.rptr_w.inp.get() == N as u8
    }
    fn empty(&self) -> bool {
        self.rptr.get() == self.wptr_r.inp.get()
    }

    /// Waits for an offer it has room for, then takes it. The pointer
    /// crosses as the value it will have after this write, so the read
    /// side counts the word as soon as its synchroniser sees it.
    async fn write(&self, push: Rx<U<8>, W>) {
        loop {
            until(W::rising, || push.peek().is_some() && !self.full()).await;
            let w = self.wptr.get();
            let v = push.recv().unwrap_or_default();
            self.mem.write(w.raw() as usize, v);
            self.wptr.set(w + 1);
            self.wptr_r.out.set(w + 1);
        }
    }

    /// Waits for a word to give and a consumer able to take it.
    async fn read(&self, pop: Tx<U<8>, R>) {
        loop {
            until(R::rising, || !self.empty() && pop.ready().to_bool()).await;
            let r = self.rptr.get();
            pop.send(self.mem.read(r.raw() as usize));
            self.rptr.set(r + 1);
            self.rptr_w.out.set(r + 1);
        }
    }
}

impl<const N: usize, W: Clock, R: Clock> Unit<Rx<U<8>, W>, Tx<U<8>, R>>
    for Fifo<N, W, R>
{
    /// Four processes: the two sides and the two synchronisers.
    async fn run(&mut self, push: Rx<U<8>, W>, pop: Tx<U<8>, R>) {
        join2(
            join2(self.write(push), self.read(pop)),
            join2(self.wptr_r.x.run(), self.rptr_w.x.run()),
        )
        .await;
    }
}

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
            let seen = self.seen.get();
            self.seen.set(seen + 1);
            println!("t={:>2} {}: pop  {}", now(), ClkR::NAME, v.raw());
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
    for _ in 0..40 {
        sim.step();
    }
}
