// SPDX-License-Identifier: Apache-2.0
//! Components: units, wires, interfaces, clocks and configurations.
//!
//! One idea runs through the wiring half. Direction is not a property of
//! a wire, because the same wire is driven at one end and read at the
//! other. A signal is therefore created as a pair of ends, the way
//! `std::sync::mpsc` yields a sender and a receiver, and a unit never
//! holds a wire, only an end. Hardware inverts mpsc's cardinality: the
//! driver is unique and readers fan out, so [`In`] is `Clone` and
//! [`Out`] is not. One driver per wire is then a move, not a rule.

use crate::types::{Bit, Transaction};
use std::cell::Cell;
use std::future::Future;
use std::marker::PhantomData;
use std::pin::Pin;
use std::rc::Rc;
use std::task::{Context, Poll};

// ---------------------------------------------------------------------
// Clocks

/// A clock domain, as a type. Which clock, not how fast: the frequency
/// is a physical fact and belongs to the configuration.
pub trait Clock {
    const NAME: &'static str;
}

/// The one clock a single-clock design has. Named, because it is a
/// clock and not an absence of one: `In<u32>` is a complete type, and it
/// does not unify with any other domain.
pub struct DefaultClock;
impl Clock for DefaultClock {
    const NAME: &'static str = "clk";
}

// ---------------------------------------------------------------------
// Wires and their ends

struct Cellf<T: Copy>(Cell<T>);

/// One wire, in a clock domain. Never held by a unit; split into ends.
pub struct Signal<T: Copy + Default, C: Clock = DefaultClock>(Rc<Cellf<T>>, PhantomData<C>);

/// One channel: a transaction plus the handshake the compiler supplies.
pub struct Chan<T: Transaction, C: Clock = DefaultClock>(Rc<Cellf<(T, bool)>>, PhantomData<C>);

/// The driving end of a wire. Not `Clone`.
pub struct Out<T: Copy, C: Clock = DefaultClock>(Rc<Cellf<T>>, PhantomData<C>);
/// The reading end of a wire. `Clone`, because fanout is free.
pub struct In<T: Copy, C: Clock = DefaultClock>(Rc<Cellf<T>>, PhantomData<C>);
/// The sending end of a channel. Not `Clone`.
pub struct Tx<T: Transaction, C: Clock = DefaultClock>(Rc<Cellf<(T, bool)>>, PhantomData<C>);
/// The receiving end of a channel. `Clone`.
pub struct Rx<T: Transaction, C: Clock = DefaultClock>(Rc<Cellf<(T, bool)>>, PhantomData<C>);

impl<T: Copy, C: Clock> Clone for In<T, C> {
    fn clone(&self) -> Self { In(self.0.clone(), PhantomData) }
}
impl<T: Transaction, C: Clock> Clone for Rx<T, C> {
    fn clone(&self) -> Self { Rx(self.0.clone(), PhantomData) }
}

impl<T: Copy, C: Clock> Out<T, C> {
    pub fn set(&self, v: T) { self.0 .0.set(v) }
}
impl<T: Copy, C: Clock> In<T, C> {
    pub fn get(&self) -> T { self.0 .0.get() }
}
impl<T: Transaction, C: Clock> Tx<T, C> {
    /// Offer a transaction. Blocking until accepted is the awaiting
    /// caller's concern; the prototype records the offer.
    pub fn send(&self, v: T) { self.0 .0.set((v, true)) }
}
impl<T: Transaction, C: Clock> Rx<T, C> {
    /// Take the pending transaction, if any.
    pub fn recv(&self) -> Option<T> {
        let (v, valid) = self.0 .0.get();
        if valid { self.0 .0.set((v, false)); Some(v) } else { None }
    }
}

/// What every member of an interface can do. `split` consumes the member
/// and returns its two ends, so the `interface!` macro never has to know
/// whether a member is a wire or a channel.
pub trait Member {
    type Driver;
    type Reader;
    fn new() -> Self;
    fn split(self) -> (Self::Driver, Self::Reader);
}

impl<T: Copy + Default, C: Clock> Member for Signal<T, C> {
    type Driver = Out<T, C>;
    type Reader = In<T, C>;
    fn new() -> Self { Signal(Rc::new(Cellf(Cell::new(T::default()))), PhantomData) }
    fn split(self) -> (Out<T, C>, In<T, C>) {
        (Out(self.0.clone(), PhantomData), In(self.0, PhantomData))
    }
}

impl<T: Transaction, C: Clock> Member for Chan<T, C> {
    type Driver = Tx<T, C>;
    type Reader = Rx<T, C>;
    fn new() -> Self { Chan(Rc::new(Cellf(Cell::new((T::default(), false)))), PhantomData) }
    fn split(self) -> (Tx<T, C>, Rx<T, C>) {
        (Tx(self.0.clone(), PhantomData), Rx(self.0, PhantomData))
    }
}

/// Create a wire and get its two ends. The whole point.
pub fn signal<T: Copy + Default, C: Clock>() -> (Out<T, C>, In<T, C>) {
    Signal::<T, C>::new().split()
}

/// Create a channel and get its two ends.
pub fn chan<T: Transaction, C: Clock>() -> (Tx<T, C>, Rx<T, C>) {
    Chan::<T, C>::new().split()
}

/// The only way from one clock domain to another. A real one is a
/// synchroniser or an asynchronous FIFO. Its type is the guarantee: a
/// signal in domain `B` can be produced from one in `A` by nothing else.
pub struct Crossing<T: Copy + Default, A: Clock, B: Clock> {
    from: In<T, A>,
    to: Out<T, B>,
}

impl<T: Copy + Default, A: Clock, B: Clock> Crossing<T, A, B> {
    pub fn new(from: In<T, A>) -> (Self, In<T, B>) {
        let (to, rx) = signal::<T, B>();
        (Crossing { from, to }, rx)
    }
    pub fn step(&self) { self.to.set(self.from.get()) }
}

// ---------------------------------------------------------------------
// State

/// A register. Interior mutability, so two processes of one unit may
/// both hold `&self` and still drive it. That is the repair for the
/// fact that two processes cannot both take `&mut self`.
pub struct Reg<T: Copy>(Cell<T>);

impl<T: Copy> Reg<T> {
    pub const fn new(v: T) -> Self { Reg(Cell::new(v)) }
    pub fn get(&self) -> T { self.0.get() }
    pub fn set(&self, v: T) { self.0.set(v) }
    /// A predicated write: a multiplexer on the enable, not a branch.
    pub fn set_if(&self, pred: Bit, v: T) {
        if pred.to_bool() { self.0.set(v) }
    }
}

// ---------------------------------------------------------------------
// Control flow on a signal

/// The multiplexer. Both arms exist in the hardware; the condition picks.
pub fn mux<T: Copy>(c: Bit, a: T, b: T) -> T {
    if c.to_bool() { a } else { b }
}

/// Predicated register writes that read as a conditional. Not `if_!`,
/// because `if` branches and this does not: both arms are in the
/// hardware at once. `when` is what Chisel and SpinalHDL call it.
///
/// A `macro_rules!` macro matches a grammar, so this one predicates a
/// list of writes and cannot reach inside an arbitrary block. That limit
/// is the argument for elaborating through a procedural macro.
#[macro_export]
macro_rules! when {
    ($c:expr => { $($rt:expr => $vt:expr);* $(;)? }
             else { $($re:expr => $ve:expr);* $(;)? }) => {{
        let __c: $crate::types::Bit = $c;
        $( $rt.set_if(__c, $vt); )*
        $( $re.set_if(__c.not(), $ve); )*
    }};
    ($c:expr => { $($rt:expr => $vt:expr);* $(;)? }) => {{
        let __c: $crate::types::Bit = $c;
        $( $rt.set_if(__c, $vt); )*
    }};
}

// ---------------------------------------------------------------------
// Units

/// Marker: a struct that is an interface.
pub trait Bus {}

/// A unit. Inputs and outputs are type parameters rather than
/// associated types, so one struct may implement this more than once
/// with different shapes. Synthesis calls `run`; a unit with several
/// processes starts one `async fn` per process and joins them.
#[allow(async_fn_in_trait)]
pub trait Module<In, Out> {
    async fn run(&mut self, inputs: In, outputs: Out);
}

/// A named build. It names the top unit as well as filling it, so it is
/// the whole of what `main` needs. A design with sub-configurations
/// names them as associated types of its own.
pub trait Config {
    type Top: Module<(), ()>;
    const NAME: &'static str;
    fn top() -> Self::Top;
}

// ---------------------------------------------------------------------
// Parallelism

/// Run two processes concurrently. A unit does not terminate, so this
/// completes only if both do.
pub struct Join2<A, B> { a: A, b: B, da: bool, db: bool }

pub fn join2<A: Future<Output = ()>, B: Future<Output = ()>>(a: A, b: B) -> Join2<A, B> {
    Join2 { a, b, da: false, db: false }
}

impl<A: Future<Output = ()>, B: Future<Output = ()>> Future for Join2<A, B> {
    type Output = ();
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        // Safety: neither field is moved out, and `self` is pinned.
        let t = unsafe { self.get_unchecked_mut() };
        if !t.da && unsafe { Pin::new_unchecked(&mut t.a) }.poll(cx).is_ready() { t.da = true }
        if !t.db && unsafe { Pin::new_unchecked(&mut t.b) }.poll(cx).is_ready() { t.db = true }
        if t.da && t.db { Poll::Ready(()) } else { Poll::Pending }
    }
}

/// Run every process in an iterator, for an array of instances.
pub async fn join_all<F: Future<Output = ()>>(fs: impl IntoIterator<Item = F>) {
    for f in fs { f.await }
}

// ---------------------------------------------------------------------
// Driving a design

fn noop_waker() -> std::task::Waker {
    use std::task::{RawWaker, RawWakerVTable, Waker};
    fn nop(_: *const ()) {}
    fn clone(_: *const ()) -> RawWaker { RawWaker::new(std::ptr::null(), &VT) }
    static VT: RawWakerVTable = RawWakerVTable::new(clone, nop, nop, nop);
    unsafe { Waker::from_raw(RawWaker::new(std::ptr::null(), &VT)) }
}

/// One cycle: one poll of a future. The prototype's whole executor.
pub fn step<F: Future<Output = ()>>(f: F) -> bool {
    let mut f = Box::pin(f);
    let w = noop_waker();
    let mut cx = Context::from_waker(&w);
    f.as_mut().poll(&mut cx).is_ready()
}

/// Elaborate the design a configuration names. A real one emits a
/// netlist; this one runs a cycle and reports, which is enough for a
/// `main` to reach the design through nothing but the configuration.
pub fn elaborate<C: Config>() -> C::Top {
    let mut top = C::top();
    step(top.run((), ()));
    top
}
