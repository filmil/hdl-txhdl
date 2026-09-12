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
use std::task::{Context, Poll, RawWaker, RawWakerVTable, Waker};

// ---------------------------------------------------------------------
// Clocks

/// A clock domain, as a type. Which clock, and how it stands to the
/// other clocks of the design: a period and a phase, in a unit common
/// to all of them. The frequency in hertz is a physical fact and
/// belongs to the configuration; the ratio and the offset between two
/// clocks are what a design depends on, and they are here.
///
/// A clock with period 3 and phase 1 has edges at 1, 4, 7, ...; the
/// default is period 1 and phase 0, so a single-clock design never
/// mentions either. This is what SDC's `create_clock -period -waveform`
/// states, in the same terms.
pub trait Clock: 'static {
    const NAME: &'static str;
    /// Ticks between rising edges. Two per cycle for the default clock,
    /// so that its falling edge is a tick of its own.
    const PERIOD: u64 = 2;
    /// The tick of the first rising edge.
    const PHASE: u64 = 0;
    /// The next rising edge of this clock: the wait a process makes
    /// once per iteration. State is read after it, plainly.
    fn rising() -> Tick
    where
        Self: Sized,
    {
        rising::<Self>()
    }
    /// The next falling edge, half a period after a rising one.
    fn falling() -> Tick
    where
        Self: Sized,
    {
        falling::<Self>()
    }
    /// Whether this clock has a rising edge at tick `t`.
    fn rising_at(t: u64) -> bool {
        t >= Self::PHASE && (t - Self::PHASE) % Self::PERIOD == 0
    }
    /// Whether this clock has a falling edge at tick `t`.
    fn falling_at(t: u64) -> bool {
        let f = Self::PHASE + Self::PERIOD / 2;
        t >= f && (t - f) % Self::PERIOD == 0
    }
    /// Whether this clock is high at tick `t`: from a rising edge up to
    /// the falling one.
    fn high_at(t: u64) -> bool {
        t >= Self::PHASE && (t - Self::PHASE) % Self::PERIOD < Self::PERIOD / 2
    }
}

/// Which edge of a clock a wait is for.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Edge {
    Rising,
    Falling,
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
pub struct Signal<T: Copy + Default, C: Clock = DefaultClock>(
    Rc<Cellf<T>>,
    PhantomData<C>,
);

/// A channel's state. An elastic buffer of two entries, so a sender
/// and a receiver that both run every cycle pass one transaction per
/// cycle with `valid` and `ready` registered on both sides and no
/// combinational path between the units; and the two wires of the
/// current step, the offer and the take, each stamped with the step
/// it belongs to, since a wire not driven this step is low. A send
/// and a receive commit at the end of the step, as a register drive
/// does, so what a process sees is the buffer as the edge left it,
/// whichever process ran first.
struct ChanCell<T: Copy> {
    head: Cell<Option<T>>,
    tail: Cell<Option<T>>,
    push: Cell<Option<T>>,
    pop: Cell<bool>,
    offered: Cell<T>,
    offer_at: Cell<u64>,
    take_at: Cell<u64>,
}

impl<T: Copy + Default> ChanCell<T> {
    fn new() -> Self {
        ChanCell {
            head: Cell::new(None),
            tail: Cell::new(None),
            push: Cell::new(None),
            pop: Cell::new(false),
            offered: Cell::new(T::default()),
            offer_at: Cell::new(u64::MAX),
            take_at: Cell::new(u64::MAX),
        }
    }
    /// Whether the sender offered at this step: the `valid` wire.
    fn offering(&self) -> bool {
        self.offer_at.get() == now()
    }
    /// Whether the receiver took at this step: the `ready` wire.
    fn taking(&self) -> bool {
        self.take_at.get() == now()
    }
}

impl<T: Copy> Commit for ChanCell<T> {
    fn apply(&self) {
        if self.pop.take() {
            self.head.set(self.tail.take());
        }
        if let Some(v) = self.push.take() {
            if self.head.get().is_none() {
                self.head.set(Some(v));
            } else {
                self.tail.set(Some(v));
            }
        }
    }
}

/// One channel: a transaction plus the handshake the compiler supplies.
pub struct Chan<T: Transaction, C: Clock = DefaultClock>(
    Rc<ChanCell<T>>,
    PhantomData<C>,
);

/// The driving end of a wire. Not `Clone`.
pub struct Out<T: Copy, C: Clock = DefaultClock>(Rc<Cellf<T>>, PhantomData<C>);
/// The reading end of a wire. `Clone`, because fanout is free.
pub struct In<T: Copy, C: Clock = DefaultClock>(Rc<Cellf<T>>, PhantomData<C>);
/// The sending end of a channel. Not `Clone`.
pub struct Tx<T: Transaction, C: Clock = DefaultClock>(
    Rc<ChanCell<T>>,
    PhantomData<C>,
);
/// The receiving end of a channel. `Clone`.
pub struct Rx<T: Transaction, C: Clock = DefaultClock>(
    Rc<ChanCell<T>>,
    PhantomData<C>,
);

impl<T: Copy, C: Clock> Clone for In<T, C> {
    fn clone(&self) -> Self {
        In(self.0.clone(), PhantomData)
    }
}
impl<T: Transaction, C: Clock> Clone for Rx<T, C> {
    fn clone(&self) -> Self {
        Rx(self.0.clone(), PhantomData)
    }
}

impl<T: Copy, C: Clock> Out<T, C> {
    pub fn set(&self, v: impl Into<T>) {
        self.0 .0.set(v.into())
    }
}
impl<T: Copy, C: Clock> In<T, C> {
    pub fn get(&self) -> T {
        self.0 .0.get()
    }
}
impl<T: Transaction, C: Clock> Tx<T, C> {
    /// Offer a transaction: `valid` high and `data` this step, into
    /// the channel at the end of it. The sender asks `ready` first;
    /// sending into a channel with no room is the bug the handshake
    /// exists to prevent.
    pub fn send(&self, v: T) {
        assert!(self.ready().to_bool(), "send on a channel with no room");
        self.0.offered.set(v);
        self.0.offer_at.set(now());
        self.0.push.set(Some(v));
        commit(self.0.clone());
    }
    /// Whether the channel has room for an offer at this step: the
    /// backpressure, as the edge left it.
    pub fn ready(&self) -> Bit {
        Bit::from_bool(self.0.tail.get().is_none())
    }
}
impl<T: Transaction, C: Clock> Rx<T, C> {
    /// The transaction at the channel's head, if any, left in place:
    /// `valid` and `data` as the edge left them.
    pub fn peek(&self) -> Option<T> {
        self.0.head.get()
    }
    /// Wait for a transaction: the next edge of the channel's clock at
    /// which the channel holds one, and take it. An event, like
    /// `C::rising()` and `until`; state is read after it.
    pub async fn wait(&self) -> T {
        loop {
            rising::<C>().await;
            if let Some(v) = self.recv() {
                return v;
            }
        }
    }
    /// Take the transaction at the head, if any: `ready` high this
    /// step, the head gone at the end of it. One take per step.
    pub fn recv(&self) -> Option<T> {
        let v = self.0.head.get()?;
        if self.0.pop.get() {
            return None;
        }
        self.0.pop.set(true);
        self.0.take_at.set(now());
        commit(self.0.clone());
        Some(v)
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
    fn new() -> Self {
        Signal(Rc::new(Cellf(Cell::new(T::default()))), PhantomData)
    }
    fn split(self) -> (Out<T, C>, In<T, C>) {
        (Out(self.0.clone(), PhantomData), In(self.0, PhantomData))
    }
}

impl<T: Transaction, C: Clock> Member for Chan<T, C> {
    type Driver = Tx<T, C>;
    type Reader = Rx<T, C>;
    fn new() -> Self {
        Chan(Rc::new(ChanCell::new()), PhantomData)
    }
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
    /// The synchroniser: samples the source at every edge of the
    /// destination clock. A process of the unit that owns the crossing;
    /// `run` joins it with the others.
    pub async fn run(&self) {
        loop {
            rising::<B>().await;
            self.to.set(self.from.get())
        }
    }
}

// ---------------------------------------------------------------------
// State

/// A register. Interior mutability, so two processes of one unit may
/// both hold `&self` and still drive it. That is the repair for the
/// fact that two processes cannot both take `&mut self`.
///
/// A register is the cycle boundary, and its interface says so. `set`
/// is a drive and is plain: it states the next value. `get` is `async`:
/// the value a register holds is the one latched at the clock edge, so
/// reading it is where a process waits for that edge, and the `.await`
/// is the mark the lowering will turn into the register. A wire has no
/// edge, which is why [`In::get`] is not `async` and this is.
pub struct Reg<T: Copy, C: Clock = DefaultClock>(
    Rc<RegCell<T>>,
    PhantomData<C>,
);

/// A second handle on the same register: what a testbench keeps to
/// read a unit's state while the unit runs.
impl<T: Copy, C: Clock> Clone for Reg<T, C> {
    fn clone(&self) -> Self {
        Reg(self.0.clone(), PhantomData)
    }
}

/// A wire a unit keeps as a field, for looking at: a `let` of the
/// loop that has a name in the trace as well as in the netlist. The
/// process drives it with `set` in the step and may read it back with
/// `get` in the same step; it holds nothing across the edge. In the
/// netlist it is the wire the `let` would have been; in a waveform
/// it shows under the unit's name like a register, which is what an
/// internal signal needs to be seen without a port for it.
pub struct Wire<T: Copy, C: Clock = DefaultClock>(Rc<Cellf<T>>, PhantomData<C>);

impl<T: Copy, C: Clock> Clone for Wire<T, C> {
    fn clone(&self) -> Self {
        Wire(self.0.clone(), PhantomData)
    }
}

impl<T: Copy + Default, C: Clock> Default for Wire<T, C> {
    fn default() -> Self {
        Wire(Rc::new(Cellf(Cell::new(T::default()))), PhantomData)
    }
}

impl<T: Copy, C: Clock> Wire<T, C> {
    pub fn set(&self, v: impl Into<T>) {
        self.0 .0.set(v.into())
    }
    pub fn get(&self) -> T {
        self.0 .0.get()
    }
}

/// What a register holds: the latched value and the pending drive.
struct RegCell<T: Copy> {
    cur: Cell<T>,
    next: Cell<Option<T>>,
}

/// A drive scheduled for the end of the step.
trait Commit {
    fn apply(&self);
}

impl<T: Copy> Commit for RegCell<T> {
    fn apply(&self) {
        if let Some(v) = self.next.take() {
            self.cur.set(v)
        }
    }
}

impl<T: Copy + Default + 'static, C: Clock> Default for Reg<T, C> {
    fn default() -> Self {
        Reg::new(T::default())
    }
}

impl<T: Copy + 'static, C: Clock> Reg<T, C> {
    pub fn new(v: impl Into<T>) -> Self {
        Reg(
            Rc::new(RegCell {
                cur: Cell::new(v.into()),
                next: Cell::new(None),
            }),
            PhantomData,
        )
    }

    /// Read the register: the value latched at the last edge. Plain,
    /// because reading is not waiting; the wait is the edge the process
    /// made before it, `C::rising()`, a channel's `wait`, or `until`.
    pub fn get(&self) -> T {
        self.0.cur.get()
    }

    /// Drive the next value. Plain, and deferred: the register takes it
    /// at the end of the step, so a read after a drive in the same
    /// iteration still sees the value the edge latched, as in hardware.
    pub fn set(&self, v: impl Into<T>) {
        self.0.next.set(Some(v.into()));
        commit(self.0.clone());
    }

    /// A predicated drive: a multiplexer on the enable, not a branch.
    pub fn set_if(&self, pred: Bit, v: impl Into<T>) {
        if pred.to_bool() {
            self.set(v)
        }
    }
}

// ---------------------------------------------------------------------
// Control flow on a signal

/// The multiplexer. Both arms exist in the hardware; the condition picks.
pub fn mux<T: Copy>(c: Bit, a: T, b: T) -> T {
    if c.to_bool() {
        a
    } else {
        b
    }
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
pub trait Unit<In, Out> {
    async fn run(&mut self, inputs: In, outputs: Out);
}

/// A named build. It names the top unit as well as filling it, so it is
/// the whole of what `main` needs. A design with sub-configurations
/// names them as associated types of its own.
pub trait Config {
    type Top: Unit<(), ()> + Default;
    const NAME: &'static str;
    /// The top unit, built from its `Default`. A design that needs
    /// anything else overrides this; most do not, because a register's
    /// default is its reset value and a socket's is its implementation.
    fn top() -> Self::Top {
        Default::default()
    }
}

/// Declare a build. Writes the config type, its design-specific
/// configuration impl, and its `Config` impl, so a build is one item.
///
/// ```ignore
/// config! { Fpga: TopConfig for Top<Fpga> {
///     type Filter = SixteenTaps;
///     const CLK_HZ: u64 = 100_000_000;
/// } }
/// ```
///
/// The body is spliced into `impl $design for $name` unchanged, so it is
/// ordinary associated items and the macro never has to parse them.
#[macro_export]
macro_rules! config {
    ($name:ident : $design:ident for $top:ty { $($body:tt)* }) => {
        #[derive(Default)]
        pub struct $name;
        impl $design for $name { $($body)* }
        impl $crate::comp::Config for $name {
            type Top = $top;
            const NAME: &'static str = stringify!($name);
        }
    };
}

/// A memory: `N` words written on the edge of `C` and read by address
/// with no wait, the way a dual-port RAM's read port is asynchronous.
/// The write is a drive, like [`Reg::set`]; the read is a wire. A
/// memory read from another clock domain is legal in the hardware and
/// safe only under a discipline the type system does not check, which
/// is what an asynchronous FIFO's pointers are for.
pub struct Mem<T: Copy, const N: usize, C: Clock = DefaultClock>(
    Rc<MemCell<T>>,
    PhantomData<C>,
);

/// On the heap, so a large memory is not built on the stack.
struct MemCell<T: Copy> {
    words: Vec<Cell<T>>,
    next: Cell<Option<(usize, T)>>,
}

impl<T: Copy> Commit for MemCell<T> {
    fn apply(&self) {
        if let Some((a, v)) = self.next.take() {
            let n = self.words.len();
            self.words[a % n].set(v)
        }
    }
}

impl<T: Copy + Default + 'static, const N: usize, C: Clock> Default
    for Mem<T, N, C>
{
    fn default() -> Self {
        Mem(
            Rc::new(MemCell {
                words: (0..N).map(|_| Cell::new(T::default())).collect(),
                next: Cell::new(None),
            }),
            PhantomData,
        )
    }
}

/// A second handle on the same memory: what a testbench keeps to read
/// a register file or a data memory while the unit runs.
impl<T: Copy, const N: usize, C: Clock> Clone for Mem<T, N, C> {
    fn clone(&self) -> Self {
        Mem(self.0.clone(), PhantomData)
    }
}

impl<T: Copy + Default + 'static, const N: usize, C: Clock> Mem<T, N, C> {
    /// A memory with its first words given: a program, a table. What
    /// a ROM is at elaboration, and what a testbench loads.
    pub fn with(words: &[T]) -> Self {
        let m = Self::default();
        for (i, w) in words.iter().enumerate().take(N) {
            m.0.words[i].set(*w);
        }
        m
    }
}

/// One word of a memory, addressed: what [`Mem::at`] gives out and a
/// predicated drive lands on, so `when!` and `case!` write a memory as
/// they write a register, `self.m.at(addr) <= value`.
pub struct Slot<T: Copy>(Rc<MemCell<T>>, usize);

impl<T: Copy + 'static> Slot<T> {
    /// Drive the word. Plain and deferred, like a register drive.
    pub fn set(&self, v: impl Into<T>) {
        self.0.next.set(Some((self.1, v.into())));
        commit(self.0.clone());
    }
    pub fn set_if(&self, pred: Bit, v: impl Into<T>) {
        if pred.to_bool() {
            self.set(v)
        }
    }
}

impl<T: Copy + 'static, const N: usize, C: Clock> Mem<T, N, C> {
    /// The word at an address, as a drive's target.
    pub fn at(&self, addr: impl Into<usize>) -> Slot<T> {
        Slot(self.0.clone(), addr.into())
    }
    /// The write port. Plain and deferred, like a register drive; one
    /// write per step, which is what one port is.
    pub fn write(&self, addr: impl Into<usize>, v: impl Into<T>) {
        self.at(addr).set(v)
    }
    /// The read port. Plain, like a wire, and as many reads as a cycle
    /// wants: a register file reads two.
    pub fn read(&self, addr: impl Into<usize>) -> T {
        self.0.words[addr.into() % N].get()
    }
}

// ---------------------------------------------------------------------
// Parallelism

/// Run two processes concurrently. A unit does not terminate, so this
/// completes only if both do.
pub struct Join2<A, B> {
    a: A,
    b: B,
    da: bool,
    db: bool,
    wa: Waker,
    wb: Waker,
}

pub fn join2<A: Future<Output = ()>, B: Future<Output = ()>>(
    a: A,
    b: B,
) -> Join2<A, B> {
    Join2 {
        a,
        b,
        da: false,
        db: false,
        wa: process(),
        wb: process(),
    }
}

impl<A: Future<Output = ()>, B: Future<Output = ()>> Future for Join2<A, B> {
    type Output = ();
    fn poll(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<()> {
        // Safety: neither field is moved out, and `self` is pinned.
        let t = unsafe { self.get_unchecked_mut() };
        // Each side is a process of its own, so each gets its own edges.
        let (mut ca, mut cb) =
            (Context::from_waker(&t.wa), Context::from_waker(&t.wb));
        if !t.da
            && unsafe { Pin::new_unchecked(&mut t.a) }
                .poll(&mut ca)
                .is_ready()
        {
            t.da = true
        }
        if !t.db
            && unsafe { Pin::new_unchecked(&mut t.b) }
                .poll(&mut cb)
                .is_ready()
        {
            t.db = true
        }
        if t.da && t.db {
            Poll::Ready(())
        } else {
            Poll::Pending
        }
    }
}

/// Run every process in an iterator concurrently, for an array of
/// instances. Polls all of them each cycle, because a process loops and
/// a sequential join would never reach the second one.
pub async fn join_all<F: Future<Output = ()>>(fs: impl IntoIterator<Item = F>) {
    let mut fs: Vec<Pin<Box<F>>> = fs.into_iter().map(Box::pin).collect();
    let mut done = vec![false; fs.len()];
    let ws: Vec<Waker> = fs.iter().map(|_| process()).collect();
    std::future::poll_fn(|_cx| {
        for (i, f) in fs.iter_mut().enumerate() {
            let mut c = Context::from_waker(&ws[i]);
            if !done[i] && f.as_mut().poll(&mut c).is_ready() {
                done[i] = true
            }
        }
        if done.iter().all(|d| *d) {
            Poll::Ready(())
        } else {
            Poll::Pending
        }
    })
    .await
}

/// Several waits at once. `parallel!(a, b, ..)` polls every future each
/// step and completes when all have, with their outputs as a tuple. Two
/// waits inside it share an edge, and the executor knows it: a wait
/// polled inside a group after another wait of the group has crossed
/// the edge is free. The same two waits written one after the other are
/// two edges, because every `.await` is a cycle boundary and a group is
/// the one way to say that two are not.
///
/// `join2` and `join_all` are the process-level counterpart: they give
/// each child its own process. `parallel!` stays in one.
#[macro_export]
macro_rules! parallel {
    ($($f:expr),+ $(,)?) => {
        $crate::comp::Join(($($crate::comp::MaybeDone::Pending($f),)+))
    };
}

/// One future inside a [`Join`]: still running, finished with its
/// output held, or its output already taken.
pub enum MaybeDone<F: Future> {
    Pending(F),
    Done(F::Output),
    Taken,
}

impl<F: Future> MaybeDone<F> {
    /// Poll if still pending. Returns whether the output is ready.
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> bool {
        // Safety: the pending future is never moved once polled here.
        let this = unsafe { self.get_unchecked_mut() };
        if let MaybeDone::Pending(f) = this {
            match unsafe { Pin::new_unchecked(f) }.poll(cx) {
                Poll::Ready(v) => *this = MaybeDone::Done(v),
                Poll::Pending => return false,
            }
        }
        true
    }
    fn take(self: Pin<&mut Self>) -> F::Output {
        let this = unsafe { self.get_unchecked_mut() };
        match std::mem::replace(this, MaybeDone::Taken) {
            MaybeDone::Done(v) => v,
            _ => panic!("parallel!: output taken before it was ready"),
        }
    }
}

/// The future `parallel!` builds: a tuple of [`MaybeDone`].
pub struct Join<T>(pub T);

macro_rules! impl_join {
    ($($F:ident $i:tt),+) => {
        impl<$($F: Future),+> Future for Join<($(MaybeDone<$F>,)+)> {
            type Output = ($($F::Output,)+);
            fn poll(
                self: Pin<&mut Self>,
                cx: &mut Context<'_>,
            ) -> Poll<Self::Output> {
                // Safety: the tuple is never moved out while pinned.
                let t = unsafe { &mut self.get_unchecked_mut().0 };
                let saved = enter_group();
                let mut all = true;
                $( all &= unsafe { Pin::new_unchecked(&mut t.$i) }.poll(cx); )+
                leave_group(saved);
                if all {
                    Poll::Ready((
                        $( unsafe { Pin::new_unchecked(&mut t.$i) }.take(), )+
                    ))
                } else {
                    Poll::Pending
                }
            }
        }
    };
}
impl_join!(A 0, B 1);
impl_join!(A 0, B 1, C 2);
impl_join!(A 0, B 1, C 2, D 3);
impl_join!(A 0, B 1, C 2, D 3, E 4);
impl_join!(A 0, B 1, C 2, D 3, E 4, G 5);

// ---------------------------------------------------------------------
// The clock edge

/// The prototype's time. One poll of the top unit is one tick, and each
/// clock has a rising edge at the ticks its period and phase say and a
/// falling edge half a period later. Every wait is built from [`Tick`]:
/// a rising or falling edge of a named clock, an operator's cycle in
/// the clock its process is in, and the waits built on those, a
/// channel's `wait` and `until`.
///
/// A process waits for an event and then reads state; reads are plain.
/// Every `.await` is a cycle boundary: a process that has crossed an
/// edge at this step cannot cross it again, so a second wait written
/// after a first waits for the next edge. Waits that share one edge are
/// written inside `parallel!`, and that is the one case
/// the executor treats specially: once one wait of the group has
/// crossed the edge at this step, the others in the group are free,
/// each once. Drives are deferred to the end of the step, so a read
/// after a drive still sees what the edge latched. Processes are told
/// apart by their waker, which [`join2`] and [`join_all`] give each
/// child fresh. A process is in the domain of the last edge it crossed;
/// the executor does not check that it stays there, and state of
/// another domain read without a [`Crossing`] is a hazard the prototype
/// runs rather than refuses.
mod clock {
    use std::any::TypeId;
    use std::cell::{Cell, RefCell};
    use std::collections::HashMap;

    /// A clock as the executor sees it.
    #[derive(Clone, Copy, PartialEq)]
    pub struct Clk {
        pub id: TypeId,
        pub period: u64,
        pub phase: u64,
    }
    impl Clk {
        pub fn edge_at(&self, t: u64, e: super::Edge) -> bool {
            let at = match e {
                super::Edge::Rising => self.phase,
                super::Edge::Falling => self.phase + self.period / 2,
            };
            t >= at && (t - at) % self.period == 0
        }
    }
    /// What the executor knows of a process: the clock it is in and the
    /// last time step it crossed an edge of it.
    #[derive(Clone, Copy)]
    pub struct Proc {
        pub clk: Clk,
        pub edge: u64,
    }

    thread_local! {
        pub static TIME: Cell<u64> = const { Cell::new(0) };
        pub static NEXT: Cell<usize> = const { Cell::new(1) };
        /// How many `parallel!` groups are being polled.
        pub static PARALLEL: Cell<u32> = const { Cell::new(0) };
        /// The step at which the innermost group crossed its edge.
        pub static GROUP_EDGE: Cell<Option<u64>> = const { Cell::new(None) };
        pub static PROCS: RefCell<HashMap<usize, Proc>> =
            RefCell::new(HashMap::new());
        /// Per process and wait: the last step it completed at, so a
        /// wait in a group completes once per edge.
        pub static DONE: RefCell<HashMap<(usize, usize), u64>> =
            RefCell::new(HashMap::new());
        /// Drives scheduled this step, applied when it ends.
        pub static COMMITS: RefCell<Vec<std::rc::Rc<dyn super::Commit>>> =
            RefCell::new(Vec::new());
        /// The trace sink, told the step number when a step ends.
        pub static TRACER: RefCell<Option<Box<dyn FnMut(u64)>>> =
            RefCell::new(None);
    }
}

/// Schedule a drive for the end of the step.
fn commit(c: Rc<dyn Commit>) {
    clock::COMMITS.with(|v| v.borrow_mut().push(c))
}

/// Enter a `parallel!` group for one poll. Returns what to hand back to
/// `leave_group`.
fn enter_group() -> Option<u64> {
    clock::PARALLEL.with(|p| p.set(p.get() + 1));
    clock::GROUP_EDGE.with(|g| g.replace(None))
}
fn leave_group(saved: Option<u64>) {
    clock::PARALLEL.with(|p| p.set(p.get() - 1));
    clock::GROUP_EDGE.with(|g| g.set(saved));
}

fn clk_of<C: Clock>() -> clock::Clk {
    clock::Clk {
        id: std::any::TypeId::of::<C>(),
        period: C::PERIOD,
        phase: C::PHASE,
    }
}

/// The current time step. For testbenches and traces.
pub fn now() -> u64 {
    clock::TIME.with(|t| t.get())
}

/// A wait for a clock edge: rising or falling, of a named clock, or,
/// for an operator's cycle, the rising edge of whatever clock the
/// process is in.
pub struct Tick {
    clk: Option<clock::Clk>,
    edge: Edge,
}

/// One cycle of the process's clock, unconditionally. Operators and
/// `cycles(n)` use this.
pub fn tick() -> Tick {
    Tick {
        clk: None,
        edge: Edge::Rising,
    }
}

/// The next rising edge of clock `C`. The wait a process makes once
/// per iteration before it reads state; `C::rising()` is the same.
pub fn rising<C: Clock>() -> Tick {
    Tick {
        clk: Some(clk_of::<C>()),
        edge: Edge::Rising,
    }
}

/// The next falling edge of clock `C`; `C::falling()` is the same.
pub fn falling<C: Clock>() -> Tick {
    Tick {
        clk: Some(clk_of::<C>()),
        edge: Edge::Falling,
    }
}

/// The next edge, of the kind `edge` makes, at which `cond` holds:
/// `until(C::rising, || ..)` or `until(C::falling, || ..)`. The
/// condition reads state, plainly, and is asked once per edge.
pub async fn until(edge: impl Fn() -> Tick, mut cond: impl FnMut() -> bool) {
    loop {
        edge().await;
        if cond() {
            return;
        }
    }
}

impl Future for Tick {
    type Output = ();
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        let p = cx.waker().data() as usize;
        let key = &*self as *const Tick as usize;
        let now = now();
        let proc_ = clock::PROCS.with(|m| m.borrow().get(&p).copied());
        // An operator's cycle is in the process's own clock; a process
        // that has crossed no edge yet is in the default clock.
        let clk = self
            .clk
            .or(proc_.map(|q| q.clk))
            .unwrap_or_else(clk_of::<DefaultClock>);
        let crossed = proc_
            .map(|q| q.clk == clk && q.edge == now)
            .unwrap_or(false);
        let grouped = clock::PARALLEL.with(|g| g.get()) > 0;
        if !crossed && clk.edge_at(now, self.edge) {
            clock::PROCS.with(|m| {
                m.borrow_mut().insert(p, clock::Proc { clk, edge: now })
            });
            if grouped {
                clock::GROUP_EDGE.with(|g| g.set(Some(now)));
            }
            clock::DONE.with(|d| d.borrow_mut().insert((p, key), now));
            return Poll::Ready(());
        }
        // Inside a group whose edge was crossed at this step, the other
        // waits of the group complete at it too, each once.
        let group_here =
            grouped && clock::GROUP_EDGE.with(|g| g.get()) == Some(now);
        let fresh =
            clock::DONE.with(|d| d.borrow().get(&(p, key)) != Some(&now));
        if group_here && crossed && fresh {
            clock::DONE.with(|d| d.borrow_mut().insert((p, key), now));
            return Poll::Ready(());
        }
        Poll::Pending
    }
}

/// A process started at this step's edge of `C`: an invocation of a
/// pipeline, whose first wait is then the next edge and not this one.
pub(crate) fn process_in<C: Clock>() -> Waker {
    let w = process();
    let p = w.data() as usize;
    let clk = clk_of::<C>();
    clock::PROCS
        .with(|m| m.borrow_mut().insert(p, clock::Proc { clk, edge: now() }));
    w
}

/// A waker that names a process. It never wakes anything, because the
/// executor polls everything every cycle; its data is the process id.
fn process() -> Waker {
    fn nop(_: *const ()) {}
    fn clone(p: *const ()) -> RawWaker {
        RawWaker::new(p, &VT)
    }
    static VT: RawWakerVTable = RawWakerVTable::new(clone, nop, nop, nop);
    let id = clock::NEXT.with(|n| {
        let i = n.get();
        n.set(i + 1);
        i
    });
    unsafe { Waker::from_raw(RawWaker::new(id as *const (), &VT)) }
}

/// End the step: apply every drive scheduled in it, then advance time.
fn advance() {
    let drives = clock::COMMITS.with(|c| std::mem::take(&mut *c.borrow_mut()));
    for d in drives {
        d.apply()
    }
    let t = now();
    clock::TRACER.with(|tr| {
        if let Some(f) = tr.borrow_mut().as_mut() {
            f(t)
        }
    });
    clock::TIME.with(|t| t.set(t.get() + 1))
}

// ---------------------------------------------------------------------
// Driving a design

/// Run a future for a number of time steps: one poll per step. Returns
/// whether it completed, which a unit never does, because a unit loops.
/// In a design with one clock of period 1, a step is a cycle.
pub fn run_for<F: Future<Output = ()>>(f: F, steps: usize) -> bool {
    let mut f = Box::pin(f);
    let w = process();
    let mut cx = Context::from_waker(&w);
    for _ in 0..steps {
        let done = f.as_mut().poll(&mut cx).is_ready();
        advance();
        if done {
            return true;
        }
    }
    false
}

/// One step.
pub fn step<F: Future<Output = ()>>(f: F) -> bool {
    run_for(f, 1)
}

/// One step with no process: the drives made so far commit. What a
/// value-level example calls between a send and a receive, since a
/// channel, like a register, shows a drive at the next edge.
pub fn settle() {
    advance()
}

/// A design being run one time step at a time, so a testbench can look
/// at its wires between steps. Clone the reading ends you want to watch
/// before starting it, because starting it borrows the top.
pub struct Running<F: Future<Output = ()>> {
    f: Pin<Box<F>>,
    w: Waker,
    done: bool,
}

impl<F: Future<Output = ()>> Running<F> {
    pub fn new(f: F) -> Self {
        Running {
            f: Box::pin(f),
            w: process(),
            done: false,
        }
    }

    /// Run the current tick, then advance. Returns whether the design
    /// has finished, which a unit never does.
    pub fn step(&mut self) -> bool {
        if self.done {
            return true;
        }
        let mut cx = Context::from_waker(&self.w);
        self.done = self.f.as_mut().poll(&mut cx).is_ready();
        advance();
        self.done
    }

    /// One cycle of the default clock: its period in ticks.
    pub fn cycle(&mut self) -> bool {
        for _ in 0..DefaultClock::PERIOD {
            if self.step() {
                return true;
            }
        }
        false
    }
}

/// Elaborate the design a configuration names and run it for a number
/// of time steps. A real one emits a netlist; this one simulates, which is
/// enough for a `main` to reach the design through nothing but the
/// configuration and observe it with `Reg::peek`.
pub fn simulate<C: Config>(steps: usize) -> C::Top {
    let mut top = C::top();
    run_for(top.run((), ()), steps);
    top
}

/// Elaborate and run one cycle.
pub fn elaborate<C: Config>() -> C::Top {
    simulate::<C>(1)
}

// ---------------------------------------------------------------------
// Tracing

/// Waveforms. A signal's name is the field it lives in, the hierarchy
/// is the nesting of units, and a testbench names what it holds when it
/// adds it. `#[derive(Trace)]` on a unit registers every field; the
/// ends of wires and channels, registers and crossings know how to
/// register themselves, and plain values register nothing. The sink is
/// VCD, which Surfer and GTKWave open; FST would be the same probes
/// with another writer.
pub mod trace {
    use super::{clock, now, Clock, Crossing, In, Mem, Out, Reg, Rx, Tx};
    use crate::types::Value;
    use std::cell::RefCell;
    use std::collections::BTreeMap;
    use std::io::Write;
    use std::marker::PhantomData;
    use std::rc::Rc;

    /// A place in the hierarchy: a dotted path.
    pub struct Scope(String);

    impl Scope {
        pub fn new(name: &str) -> Self {
            Scope(name.to_string())
        }
        pub fn child(&self, name: &str) -> Scope {
            Scope(format!("{}.{}", self.0, name))
        }
        pub fn path(&self) -> &str {
            &self.0
        }
    }

    /// What a probe is on: state, or one end of a wire or channel. A
    /// netlist needs the kind; a waveform does not.
    #[derive(Clone, Copy, PartialEq, Eq, Debug)]
    pub enum Kind {
        Reg,
        Out,
        In,
        Tx,
        Rx,
        /// A memory: state with an address, untraced.
        Mem,
        /// A wire kept as a field, for looking at.
        Wire,
    }

    /// One traced signal: where it is, how wide, what it is, which
    /// shared cell it is on (so the two ends of one wire match), and
    /// how to read it.
    pub struct Probe {
        pub path: String,
        pub width: usize,
        pub kind: Kind,
        pub cell: usize,
        pub sample: Box<dyn Fn() -> String>,
        /// For an enum, its variants by index, so a viewer can name
        /// the value.
        pub names: Option<&'static [&'static str]>,
    }

    thread_local! {
        static PROBES: RefCell<Vec<Probe>> = const { RefCell::new(Vec::new()) };
    }

    /// Register a signal. What the `Traceable` impls call.
    pub fn probe(
        scope: &Scope,
        width: usize,
        kind: Kind,
        cell: usize,
        sample: Box<dyn Fn() -> String>,
    ) {
        probe_named(scope, width, kind, cell, sample, None)
    }

    /// Register a signal with the names of its values, if it has them.
    pub fn probe_named(
        scope: &Scope,
        width: usize,
        kind: Kind,
        cell: usize,
        sample: Box<dyn Fn() -> String>,
        names: Option<&'static [&'static str]>,
    ) {
        PROBES.with(|p| {
            p.borrow_mut().push(Probe {
                path: scope.0.clone(),
                width,
                kind,
                cell,
                sample,
                names,
            })
        })
    }

    /// Run a walk on an empty registry and hand back what it
    /// registered, leaving whatever was there before. The netlist uses
    /// it.
    pub fn collect(name: &str, t: &impl Traceable) -> Vec<Probe> {
        let saved = PROBES.with(|p| std::mem::take(&mut *p.borrow_mut()));
        t.trace(&Scope::new(name));
        PROBES.with(|p| std::mem::replace(&mut *p.borrow_mut(), saved))
    }

    /// Something with signals to register under a scope.
    pub trait Traceable {
        fn trace(&self, scope: &Scope);
    }

    impl<T: Value + 'static, C: Clock> Traceable for Reg<T, C> {
        fn trace(&self, scope: &Scope) {
            let r = self.0.clone();
            let cell = Rc::as_ptr(&self.0) as usize;
            let f = Box::new(move || r.cur.get().vcd());
            probe_named(scope, T::WIDTH, Kind::Reg, cell, f, T::names());
            let r = self.0.clone();
            parts(scope, Kind::Reg, cell, move || r.cur.get());
        }
    }

    /// A compound value is also one signal per field, under the value's
    /// name, so a viewer shows a struct as its fields.
    fn parts<T: Value + 'static>(
        scope: &Scope,
        kind: Kind,
        cell: usize,
        get: impl Fn() -> T + Clone + 'static,
    ) {
        for (i, part) in get().parts().into_iter().enumerate() {
            let g = get.clone();
            let f = Box::new(move || g().parts()[i].bits.clone());
            let s = scope.child(part.name);
            probe_named(&s, part.width, kind, cell, f, part.names);
        }
    }
    impl<T: Value + 'static, C: Clock> Traceable for In<T, C> {
        fn trace(&self, scope: &Scope) {
            let c = self.0.clone();
            let cell = Rc::as_ptr(&self.0) as usize;
            let f = Box::new(move || c.0.get().vcd());
            probe_named(scope, T::WIDTH, Kind::In, cell, f, T::names());
            let c = self.0.clone();
            parts(scope, Kind::In, cell, move || c.0.get());
        }
    }
    impl<T: Value + 'static, C: Clock> Traceable for Out<T, C> {
        fn trace(&self, scope: &Scope) {
            let c = self.0.clone();
            let cell = Rc::as_ptr(&self.0) as usize;
            let f = Box::new(move || c.0.get().vcd());
            probe_named(scope, T::WIDTH, Kind::Out, cell, f, T::names());
            let c = self.0.clone();
            parts(scope, Kind::Out, cell, move || c.0.get());
        }
    }
    /// A wire field traces as an output does: a wire, under the unit.
    impl<T: Value + 'static, C: Clock> Traceable for super::Wire<T, C> {
        fn trace(&self, scope: &Scope) {
            let c = self.0.clone();
            let cell = Rc::as_ptr(&self.0) as usize;
            let f = Box::new(move || c.0.get().vcd());
            probe_named(scope, T::WIDTH, Kind::Out, cell, f, T::names());
            let c = self.0.clone();
            parts(scope, Kind::Out, cell, move || c.0.get());
        }
    }
    /// A channel is two signals: the transaction and its valid bit.
    #[rustfmt::skip]
    impl<T: Value + super::Transaction + 'static, C: Clock> Traceable
        for Rx<T, C>
    {
        fn trace(&self, scope: &Scope) {
            channel(&self.0, scope, Kind::Rx)
        }
    }
    #[rustfmt::skip]
    impl<T: Value + super::Transaction + 'static, C: Clock> Traceable
        for Tx<T, C>
    {
        fn trace(&self, scope: &Scope) {
            channel(&self.0, scope, Kind::Tx)
        }
    }
    /// A channel is six signals: on the receiver's side the head of
    /// the buffer, `rx_data` and `rx_valid`, and the take, `rx_ready`;
    /// on the sender's side the offer, `tx_data` and `tx_valid`, and
    /// the room, `tx_ready`. Each side is what a lowered unit's port
    /// of that kind sees and drives.
    fn channel<T: Value + Copy + Default + 'static>(
        c: &Rc<super::ChanCell<T>>,
        scope: &Scope,
        kind: Kind,
    ) {
        let cell = Rc::as_ptr(c) as usize;
        let head =
            |c: &Rc<super::ChanCell<T>>| c.head.get().unwrap_or_default();
        let (a, b, d) = (c.clone(), c.clone(), c.clone());
        let rx_data = Box::new(move || head(&a).vcd());
        let rx_valid = Box::new(move || b.head.get().is_some().vcd());
        let rx_ready = Box::new(move || d.taking().vcd());
        let s = scope.child("rx_data");
        probe_named(&s, T::WIDTH, kind, cell, rx_data, T::names());
        probe(&scope.child("rx_valid"), 1, kind, cell, rx_valid);
        probe(&scope.child("rx_ready"), 1, kind, cell, rx_ready);
        let p = c.clone();
        parts(&scope.child("rx_data"), kind, cell, move || head(&p));
        let (a, b, d) = (c.clone(), c.clone(), c.clone());
        let tx_data = Box::new(move || a.offered.get().vcd());
        let tx_valid = Box::new(move || b.offering().vcd());
        let tx_ready = Box::new(move || d.tail.get().is_none().vcd());
        let s = scope.child("tx_data");
        probe_named(&s, T::WIDTH, kind, cell, tx_data, T::names());
        probe(&scope.child("tx_valid"), 1, kind, cell, tx_valid);
        probe(&scope.child("tx_ready"), 1, kind, cell, tx_ready);
        let p = c.clone();
        parts(&scope.child("tx_data"), kind, cell, move || p.offered.get());
    }
    impl<T: Value + Default + 'static, A: Clock, B: Clock> Traceable
        for Crossing<T, A, B>
    {
        fn trace(&self, scope: &Scope) {
            self.from.trace(&scope.child("from"));
            self.to.trace(&scope.child("to"));
        }
    }
    /// A memory is not traced; its ports are.
    impl<T: Copy, const N: usize, C: Clock> Traceable for Mem<T, N, C> {
        fn trace(&self, _: &Scope) {}
    }
    /// Plain values in a unit are constants, and register nothing.
    macro_rules! untraced {
        ($($t:ty),*) => {
            $( impl Traceable for $t {
                fn trace(&self, _: &Scope) {}
            } )*
        };
    }
    untraced!(u8, u16, u32, u64, u128, usize, bool, &'static str);
    untraced!(crate::types::Bit, crate::types::Logic);
    impl<const N: usize> Traceable for crate::types::U<N> {
        fn trace(&self, _: &Scope) {}
    }
    impl<T> Traceable for PhantomData<T> {
        fn trace(&self, _: &Scope) {}
    }

    /// A VCD writer. Add what to watch, then `start`; from then on every
    /// tick writes its changes, at the tick's end, when the values the
    /// tick drove have committed. A clock is a level, high from its
    /// rising edge to its falling one.
    pub struct Vcd {
        out: Box<dyn Write>,
        clocks: Vec<(String, fn(u64) -> bool)>,
    }

    impl Vcd {
        pub fn new(out: impl Write + 'static) -> Self {
            Vcd {
                out: Box::new(out),
                clocks: Vec::new(),
            }
        }
        /// A writer on the file `TXHDL_VCD` names, or none. An example
        /// that prints keeps printing; the document build sets the
        /// variable and takes the file as well.
        pub fn from_env() -> Option<Vcd> {
            let path = std::env::var("TXHDL_VCD").ok()?;
            let f = std::fs::File::create(&path).expect("TXHDL_VCD file");
            Some(Vcd::new(std::io::BufWriter::new(f)))
        }
        /// Trace a clock, as the one-bit signal it is: high from a
        /// rising edge to the falling one.
        pub fn clock<C: Clock>(&mut self) {
            self.clocks.push((C::NAME.to_string(), C::high_at));
        }
        /// Trace something under a name of the testbench's choosing.
        pub fn add(&mut self, name: &str, t: &impl Traceable) {
            t.trace(&Scope::new(name))
        }
        /// Write the header and start recording.
        pub fn start(mut self) {
            let probes: Vec<Probe> =
                PROBES.with(|p| std::mem::take(&mut *p.borrow_mut()));
            let ids: Vec<String> =
                (0..self.clocks.len() + probes.len()).map(id).collect();
            let (cids, pids) = ids.split_at(self.clocks.len());
            let (cids, pids) = (cids.to_vec(), pids.to_vec());
            let o = &mut self.out;
            writeln!(o, "$timescale 1ns $end").unwrap();
            writeln!(o, "$scope module clocks $end").unwrap();
            for ((name, _), id) in self.clocks.iter().zip(&cids) {
                writeln!(o, "$var wire 1 {id} {name} $end").unwrap();
            }
            writeln!(o, "$upscope $end").unwrap();
            let mut tree = Node::default();
            for (i, p) in probes.iter().enumerate() {
                tree.insert(&p.path, i);
            }
            tree.write(o, &probes, &pids);
            writeln!(o, "$enddefinitions $end").unwrap();
            // Initial values.
            let mut last: Vec<String> = Vec::new();
            writeln!(o, "$dumpvars").unwrap();
            for id in &cids {
                writeln!(o, "0{id}").unwrap();
            }
            for (p, id) in probes.iter().zip(&pids) {
                let v = (p.sample)();
                write_value(o, p.width, &v, id);
                last.push(v);
            }
            writeln!(o, "$end").unwrap();
            let clocks = std::mem::take(&mut self.clocks);
            let mut clast: Vec<bool> = vec![false; clocks.len()];
            let mut out = self.out;
            clock::TRACER.with(|tr| {
                *tr.borrow_mut() = Some(Box::new(move |t: u64| {
                    // The tick's end: the clocks as they stand and every
                    // value the tick's drives committed.
                    writeln!(out, "#{t}").unwrap();
                    for (i, ((_, high), id)) in
                        clocks.iter().zip(&cids).enumerate()
                    {
                        let h = high(t);
                        if h != clast[i] {
                            writeln!(out, "{}{id}", if h { 1 } else { 0 })
                                .unwrap();
                            clast[i] = h;
                        }
                    }
                    for (i, (p, id)) in probes.iter().zip(&pids).enumerate() {
                        let v = (p.sample)();
                        if v != last[i] {
                            write_value(&mut out, p.width, &v, id);
                            last[i] = v;
                        }
                    }
                }))
            });
            let _ = now;
        }
    }

    /// Stop recording, so the sink is finished and flushed. A VCD needs
    /// only dropping; an FST is told to write its tail first.
    pub fn stop() {
        clock::TRACER.with(|tr| {
            if let Some(f) = tr.borrow_mut().as_mut() {
                f(u64::MAX)
            }
        });
        clock::TRACER.with(|tr| *tr.borrow_mut() = None)
    }

    fn write_value(o: &mut dyn Write, width: usize, v: &str, id: &str) {
        if width == 1 {
            writeln!(o, "{v}{id}").unwrap()
        } else {
            writeln!(o, "b{v} {id}").unwrap()
        }
    }

    /// A VCD identifier: printable ASCII from `!`, base 94.
    fn id(mut i: usize) -> String {
        let mut s = String::new();
        loop {
            s.insert(0, (b'!' + (i % 94) as u8) as char);
            i /= 94;
            if i == 0 {
                return s;
            }
        }
    }

    /// The scope tree, built from dotted paths.
    #[derive(Default)]
    struct Node {
        children: BTreeMap<String, Node>,
        vars: Vec<usize>,
    }

    impl Node {
        fn insert(&mut self, path: &str, i: usize) {
            let mut node = self;
            let parts: Vec<&str> = path.split('.').collect();
            for part in &parts[..parts.len() - 1] {
                node = node.children.entry(part.to_string()).or_default();
            }
            node.vars.push(i);
        }
        fn write(&self, o: &mut dyn Write, probes: &[Probe], ids: &[String]) {
            for &i in &self.vars {
                let leaf = probes[i].path.rsplit('.').next().unwrap();
                writeln!(
                    o,
                    "$var wire {} {} {leaf} $end",
                    probes[i].width, ids[i]
                )
                .unwrap();
            }
            for (name, child) in &self.children {
                writeln!(o, "$scope module {name} $end").unwrap();
                child.write(o, probes, ids);
                writeln!(o, "$upscope $end").unwrap();
            }
        }
    }

    /// An FST writer: the same probes as [`Vcd`], compressed, which the
    /// viewer and the converters read as readily as VCD. `Wave::from_env`
    /// picks it when `TXHDL_FST` names a file. A compound value is one
    /// bit vector, and one signal per field beside it; the format's
    /// string variables are not in the writer used.
    pub struct Fst {
        path: String,
        clocks: Vec<(String, fn(u64) -> bool)>,
    }

    impl Fst {
        pub fn new(path: impl Into<String>) -> Self {
            Fst {
                path: path.into(),
                clocks: Vec::new(),
            }
        }
        pub fn clock<C: Clock>(&mut self) {
            self.clocks.push((C::NAME.to_string(), C::high_at));
        }
        pub fn add(&mut self, name: &str, t: &impl Traceable) {
            t.trace(&Scope::new(name))
        }
        /// Write the header and start recording. `stop` finishes the
        /// file; a process that ends without it loses the tail.
        pub fn start(self) {
            use fst_writer::{
                open_fst, FstFileType, FstInfo, FstScopeType, FstSignalType,
                FstVarDirection, FstVarType,
            };
            let probes: Vec<Probe> =
                PROBES.with(|p| std::mem::take(&mut *p.borrow_mut()));
            let info = FstInfo {
                start_time: 0,
                timescale_exponent: -9,
                version: "txhdl".to_string(),
                date: String::new(),
                file_type: FstFileType::Verilog,
            };
            let mut h = open_fst(&self.path, &info).expect("open FST");
            // The names of enum values, beside the file: the format's
            // writer has no enum tables, and a viewer or a drawing can
            // read this instead.
            let names: String = probes
                .iter()
                .filter_map(|p| {
                    p.names.map(|n| format!("{}\t{}\n", p.path, n.join(",")))
                })
                .collect();
            std::fs::write(format!("{}.names", self.path), names)
                .expect("names");
            h.scope("clocks", "", FstScopeType::Module).expect("scope");
            let mut cids = Vec::new();
            for (name, _) in &self.clocks {
                let id = h
                    .var(
                        name,
                        FstSignalType::bit_vec(1),
                        FstVarType::Wire,
                        FstVarDirection::Implicit,
                        None,
                    )
                    .expect("var");
                cids.push(id);
            }
            h.up_scope().expect("upscope");
            let mut tree = Node::default();
            for (i, p) in probes.iter().enumerate() {
                tree.insert(&p.path, i);
            }
            let mut pids = vec![None; probes.len()];
            tree.declare(&mut h, &probes, &mut pids);
            let mut body = h.finish().expect("header");
            body.time_change(0).expect("time");
            let mut last: Vec<String> = Vec::new();
            for (p, id) in probes.iter().zip(&pids) {
                let v = (p.sample)();
                body.signal_change(id.unwrap(), v.as_bytes())
                    .expect("change");
                last.push(v);
            }
            for id in &cids {
                body.signal_change(*id, b"0").expect("change");
            }
            let mut clast = vec![false; self.clocks.len()];
            let clocks = self.clocks;
            let mut body = Some(body);
            // The time table's length, and the last time in it. A table
            // of exactly twelve entries comes out unreadable from the
            // writer used (its issue is filed), so one empty step is
            // added at the end when it would have that many.
            let mut entries: u64 = 1;
            let mut last_t: u64 = 0;
            clock::TRACER.with(|tr| {
                *tr.borrow_mut() = Some(Box::new(move |t: u64| {
                    let Some(b) = body.as_mut() else { return };
                    if t == u64::MAX {
                        if entries == 12 {
                            b.time_change(last_t + 1).expect("time");
                        }
                        body.take().unwrap().finish().expect("finish");
                        return;
                    }
                    // The table already holds time 0, from the
                    // initial values; the first tick adds nothing new.
                    if t != last_t {
                        b.time_change(t).expect("time");
                        entries += 1;
                        last_t = t;
                    }
                    let it = clocks.iter().zip(&cids).enumerate();
                    for (i, ((_, high), id)) in it {
                        let hi = high(t);
                        if hi != clast[i] {
                            let v: &[u8] = if hi { b"1" } else { b"0" };
                            b.signal_change(*id, v).expect("change");
                            clast[i] = hi;
                        }
                    }
                    for (i, (p, id)) in probes.iter().zip(&pids).enumerate() {
                        let v = (p.sample)();
                        if v != last[i] {
                            b.signal_change(id.unwrap(), v.as_bytes())
                                .expect("change");
                            last[i] = v;
                        }
                    }
                }))
            });
        }
    }

    impl Node {
        fn declare<W: std::io::Write + std::io::Seek>(
            &self,
            h: &mut fst_writer::FstHeaderWriter<W>,
            probes: &[Probe],
            ids: &mut [Option<fst_writer::FstSignalId>],
        ) {
            use fst_writer::{
                FstScopeType, FstSignalType, FstVarDirection, FstVarType,
            };
            for &i in &self.vars {
                let leaf = probes[i].path.rsplit('.').next().unwrap();
                let tpe = match probes[i].kind {
                    Kind::Reg => FstVarType::Reg,
                    _ => FstVarType::Wire,
                };
                let id = h
                    .var(
                        leaf,
                        FstSignalType::bit_vec(probes[i].width as u32),
                        tpe,
                        FstVarDirection::Implicit,
                        None,
                    )
                    .expect("var");
                ids[i] = Some(id);
            }
            for (name, child) in &self.children {
                h.scope(name, "", FstScopeType::Module).expect("scope");
                child.declare(h, probes, ids);
                h.up_scope().expect("upscope");
            }
        }
    }

    /// Either sink, chosen by the environment: `TXHDL_FST` names an FST
    /// file, `TXHDL_VCD` a VCD one. An example names what to watch on
    /// whichever it gets, and calls [`stop`] when it is done.
    pub enum Wave {
        Vcd(Vcd),
        Fst(Fst),
    }

    impl Wave {
        pub fn from_env() -> Option<Wave> {
            if let Ok(p) = std::env::var("TXHDL_FST") {
                return Some(Wave::Fst(Fst::new(p)));
            }
            Vcd::from_env().map(Wave::Vcd)
        }
        pub fn clock<C: Clock>(&mut self) {
            match self {
                Wave::Vcd(v) => v.clock::<C>(),
                Wave::Fst(f) => f.clock::<C>(),
            }
        }
        pub fn add(&mut self, name: &str, t: &impl Traceable) {
            match self {
                Wave::Vcd(v) => v.add(name, t),
                Wave::Fst(f) => f.add(name, t),
            }
        }
        pub fn start(self) {
            match self {
                Wave::Vcd(v) => v.start(),
                Wave::Fst(f) => f.start(),
            }
        }
    }
}
