// Probe 12. A unit with submodules.
//
// Three questions the design had not answered:
//   a. How is a submodule instantiated?
//   b. How are two submodules wired to each other?
//   c. Does the E0499 finding from probe 5b bite again here?
//
// (c) is the interesting one. Probe 5b showed that two *processes of one
// unit* cannot both take `&mut self`. A submodule is a different case: it
// is a field, and Rust allows simultaneous mutable borrows of disjoint
// fields. So the earlier constraint does not apply, and this compiles.

use core::cell::Cell;
use core::future::Future;
use core::pin::Pin;
use core::task::{Context, Poll};

pub mod comp {
    #[allow(async_fn_in_trait)]
    pub trait Module<In, Out> {
        async fn run(&mut self, inputs: In, outputs: Out);
    }

    pub trait Config {
        type Mac: super::MacOp;
        const DEPTH: usize;
    }
}

use comp::{Config, Module};

// --- a join over a fixed number of futures ---------------------------

pub struct Join2<A, B> { a: A, b: B, da: bool, db: bool }

pub fn join2<A: Future<Output = ()>, B: Future<Output = ()>>(a: A, b: B) -> Join2<A, B> {
    Join2 { a, b, da: false, db: false }
}

impl<A: Future<Output = ()>, B: Future<Output = ()>> Future for Join2<A, B> {
    type Output = ();
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        let t = unsafe { self.get_unchecked_mut() };
        if !t.da && unsafe { Pin::new_unchecked(&mut t.a) }.poll(cx).is_ready() { t.da = true; }
        if !t.db && unsafe { Pin::new_unchecked(&mut t.b) }.poll(cx).is_ready() { t.db = true; }
        if t.da && t.db { Poll::Ready(()) } else { Poll::Pending }
    }
}

/// A join over a slice of identical futures, for an array of instances.
pub async fn join_all<F: Future<Output = ()>>(fs: impl IntoIterator<Item = F>) {
    for f in fs { f.await; }
}

// --- the wiring ------------------------------------------------------

/// A channel. Interior mutability, so both ends may hold a shared
/// reference to the same wire.
pub struct Chan<T: Copy + Default> {
    v: Cell<T>,
    full: Cell<bool>,
}

impl<T: Copy + Default> Chan<T> {
    pub const fn new() -> Self { Chan { v: Cell::new(unsafe_default()), full: Cell::new(false) } }
    pub fn send(&self, x: T) { self.v.set(x); self.full.set(true); }
    pub fn recv(&self) -> Option<T> {
        if self.full.get() { self.full.set(false); Some(self.v.get()) } else { None }
    }
}

const fn unsafe_default<T: Copy + Default>() -> T {
    // `Default::default()` is not const; a zeroed Cell is enough here and
    // the probe only needs the types to line up.
    unsafe { core::mem::zeroed() }
}

/// A bus is a struct of channels, owned by whichever unit contains both
/// ends of it.
pub struct DataBus {
    pub data: Chan<u32>,
}

impl DataBus {
    pub const fn new() -> Self { DataBus { data: Chan::new() } }
}

// --- leaf units ------------------------------------------------------

/// Ports arrive as the `run` parameters. That is what makes the wiring a
/// call rather than a stored back-reference, and it is why `Module` takes
/// inputs and outputs as parameters at all.
pub struct ProducerOut<'a> { pub out: &'a DataBus }
pub struct ConsumerIn<'a> { pub inp: &'a DataBus }

pub struct Producer { pub next: Cell<u32> }
pub struct Consumer { pub total: Cell<u32> }

impl<'a> Module<(), ProducerOut<'a>> for Producer {
    async fn run(&mut self, _i: (), o: ProducerOut<'a>) {
        let v = self.next.get();
        o.out.data.send(v);
        self.next.set(v + 1);
    }
}

impl<'a> Module<ConsumerIn<'a>, ()> for Consumer {
    async fn run(&mut self, i: ConsumerIn<'a>, _o: ()) {
        if let Some(v) = i.inp.data.recv() {
            self.total.set(self.total.get() + v);
        }
    }
}

// --- a processing element, for the array case ------------------------

pub trait MacOp { fn new() -> Self; fn step(&self, x: u32) -> u32; }

pub struct Pe { pub acc: Cell<u32> }

impl MacOp for Pe {
    fn new() -> Self { Pe { acc: Cell::new(0) } }
    fn step(&self, x: u32) -> u32 { self.acc.set(self.acc.get() + x); self.acc.get() }
}

impl Module<u32, ()> for Pe {
    async fn run(&mut self, i: u32, _o: ()) { self.step(i); }
}

// --- the parent ------------------------------------------------------

/// Submodules are fields. The bus that joins two of them is a field too,
/// owned by the unit that contains both ends.
pub struct Top<C: Config> {
    pub producer: Producer,
    pub consumer: Consumer,
    pub link: DataBus,
    pub pes: [Pe; 4],
    pub _c: core::marker::PhantomData<C>,
}

impl<C: Config> Top<C> {
    pub fn new() -> Self {
        Top {
            producer: Producer { next: Cell::new(0) },
            consumer: Consumer { total: Cell::new(0) },
            link: DataBus::new(),
            pes: [Pe::new(), Pe::new(), Pe::new(), Pe::new()],
            _c: core::marker::PhantomData,
        }
    }
}

impl<C: Config> Module<(), ()> for Top<C> {
    async fn run(&mut self, _i: (), _o: ()) {
        // Two submodules, run in parallel, wired by being handed the same
        // bus. `&mut self.producer` and `&mut self.consumer` are disjoint
        // fields, so both mutable borrows are legal at once; `&self.link`
        // is shared by both, which is what a wire is.
        join2(
            self.producer.run((), ProducerOut { out: &self.link }),
            self.consumer.run(ConsumerIn { inp: &self.link }, ()),
        )
        .await;

        // An array of instances: one future per element, joined.
        join_all(self.pes.iter_mut().enumerate().map(|(i, pe)| pe.run(i as u32, ()))).await;
    }
}

pub fn build<C: Config>() -> Top<C> { Top::<C>::new() }
