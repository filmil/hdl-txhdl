// Probe 20. A clock domain in the signal's type, after RHDL.
//
//   Signal<T, C: Clock = DefaultClock>
//
// `Clock` is a trait and every clock domain is a type implementing it.
// The default is a named type, not `()`, and the reason is what the
// default means. It is not "no clock". It is the one clock a single-clock
// design has, so that such a design never mentions a domain at all and a
// two-clock design mentions exactly the second one. `()` would say
// "nothing", and a register cannot live in nothing.
//
// The default is a domain like any other: it does not unify with
// `Clk400`, and getting from one to the other takes a crossing.

use std::cell::Cell;
use std::marker::PhantomData;
use std::rc::Rc;

pub trait Wire: Copy + Default {}
impl Wire for u32 {}
impl Wire for bool {}

/// A clock domain. A marker: which clock, not how fast. The frequency is
/// a physical fact and belongs to the configuration, which maps each
/// clock type onto a pin and a rate.
pub trait Clock {
    const NAME: &'static str;
}

/// The one clock a single-clock design has. Named, because it is a
/// clock, and a signal in it is a complete type: `Signal<u32>`.
pub struct DefaultClock;
impl Clock for DefaultClock { const NAME: &'static str = "clk"; }

pub struct Clk400;
impl Clock for Clk400 { const NAME: &'static str = "clk400"; }

struct Cellf<T: Wire>(Cell<T>);

pub struct Out<T: Wire, C: Clock = DefaultClock>(Rc<Cellf<T>>, PhantomData<C>);
pub struct In<T: Wire, C: Clock = DefaultClock>(Rc<Cellf<T>>, PhantomData<C>);

impl<T: Wire, C: Clock> Clone for In<T, C> {
    fn clone(&self) -> Self { In(self.0.clone(), PhantomData) }
}

impl<T: Wire, C: Clock> Out<T, C> { pub fn set(&self, v: T) { self.0 .0.set(v) } }
impl<T: Wire, C: Clock> In<T, C> { pub fn get(&self) -> T { self.0 .0.get() } }

pub fn signal<T: Wire, C: Clock>() -> (Out<T, C>, In<T, C>) {
    let w = Rc::new(Cellf(Cell::new(T::default())));
    (Out(w.clone(), PhantomData), In(w, PhantomData))
}

/// The only way from one clock to another. A real one is a synchroniser
/// or an asynchronous FIFO; this one records that a crossing happened.
pub struct Crossing<T: Wire, A: Clock, B: Clock> {
    from: In<T, A>,
    to: Out<T, B>,
}

impl<T: Wire, A: Clock, B: Clock> Crossing<T, A, B> {
    pub fn new(from: In<T, A>) -> (Self, In<T, B>) {
        let (to, rx) = signal::<T, B>();
        (Crossing { from, to }, rx)
    }
    pub fn step(&self) { self.to.set(self.from.get()) }
}

// --- a single-clock design never says which clock -------------------

pub struct Counter { pub out: Out<u32>, pub n: Cell<u32> }
pub struct Sampler { pub inp: In<u32>, pub seen: Cell<u32> }

impl Counter {
    pub fn step(&self) { let n = self.n.get(); self.out.set(n); self.n.set(n + 1) }
}

pub fn single_clock() -> (Counter, Sampler) {
    let (tx, rx) = signal::<u32, DefaultClock>();
    (Counter { out: tx, n: Cell::new(0) }, Sampler { inp: rx, seen: Cell::new(0) })
}

// --- a two-clock design names exactly the second one -----------------

pub struct Dsp { pub inp: In<u32, Clk400>, pub acc: Cell<u32> }

pub fn two_clocks() -> (Counter, Crossing<u32, DefaultClock, Clk400>, Dsp) {
    let (tx, rx) = signal::<u32, DefaultClock>();
    let (xing, rx400) = Crossing::<u32, DefaultClock, Clk400>::new(rx);
    (Counter { out: tx, n: Cell::new(0) }, xing, Dsp { inp: rx400, acc: Cell::new(0) })
}

// --- a unit that does not care says so ------------------------------

pub struct Probe<C: Clock> { pub inp: In<u32, C> }
impl<C: Clock> Probe<C> { pub fn which(&self) -> &'static str { C::NAME } }
