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
    /// Time steps between rising edges.
    const PERIOD: u64 = 1;
    /// Time step of the first rising edge.
    const PHASE: u64 = 0;
    /// The next rising edge of this clock: the wait a process makes
    /// once per iteration. State is read after it, plainly.
    fn edge() -> Tick
    where
        Self: Sized,
    {
        edge::<Self>()
    }
    /// Whether this clock has a rising edge at time step `t`.
    fn edge_at(t: u64) -> bool {
        t >= Self::PHASE && (t - Self::PHASE) % Self::PERIOD == 0
    }
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

/// One channel: a transaction plus the handshake the compiler supplies.
pub struct Chan<T: Transaction, C: Clock = DefaultClock>(
    Rc<Cellf<(T, bool)>>,
    PhantomData<C>,
);

/// The driving end of a wire. Not `Clone`.
pub struct Out<T: Copy, C: Clock = DefaultClock>(Rc<Cellf<T>>, PhantomData<C>);
/// The reading end of a wire. `Clone`, because fanout is free.
pub struct In<T: Copy, C: Clock = DefaultClock>(Rc<Cellf<T>>, PhantomData<C>);
/// The sending end of a channel. Not `Clone`.
pub struct Tx<T: Transaction, C: Clock = DefaultClock>(
    Rc<Cellf<(T, bool)>>,
    PhantomData<C>,
);
/// The receiving end of a channel. `Clone`.
pub struct Rx<T: Transaction, C: Clock = DefaultClock>(
    Rc<Cellf<(T, bool)>>,
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
    /// Offer a transaction. The channel holds one until it is received,
    /// so the sender asks `ready` first; sending over an offer that has
    /// not been taken is the bug the handshake exists to prevent.
    pub fn send(&self, v: T) {
        assert!(
            self.ready().to_bool(),
            "send on a channel whose last offer was not received"
        );
        self.0 .0.set((v, true))
    }
    /// Whether the channel can take an offer: the backpressure.
    pub fn ready(&self) -> Bit {
        Bit::from_bool(!self.0 .0.get().1)
    }
}
impl<T: Transaction, C: Clock> Rx<T, C> {
    /// The pending transaction, if any, left in place. The valid side
    /// of the handshake.
    pub fn peek(&self) -> Option<T> {
        let (v, valid) = self.0 .0.get();
        if valid {
            Some(v)
        } else {
            None
        }
    }
    /// Wait for a transaction: the next edge of the channel's clock at
    /// which one is offered, and take it. An event, like `C::edge()`
    /// and `until`; state is read after it.
    pub async fn wait(&self) -> T {
        loop {
            edge::<C>().await;
            if let Some(v) = self.recv() {
                return v;
            }
        }
    }
    /// Take the pending transaction, if any. Taking is the accept.
    pub fn recv(&self) -> Option<T> {
        let (v, valid) = self.0 .0.get();
        if valid {
            self.0 .0.set((v, false));
            Some(v)
        } else {
            None
        }
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
        Chan(
            Rc::new(Cellf(Cell::new((T::default(), false)))),
            PhantomData,
        )
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
            edge::<B>().await;
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
    /// made before it, `C::edge()`, a channel's `wait`, or `until`.
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
pub trait Module<In, Out> {
    async fn run(&mut self, inputs: In, outputs: Out);
}

/// A named build. It names the top unit as well as filling it, so it is
/// the whole of what `main` needs. A design with sub-configurations
/// names them as associated types of its own.
pub trait Config {
    type Top: Module<(), ()> + Default;
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
    Rc<MemCell<T, N>>,
    PhantomData<C>,
);

struct MemCell<T: Copy, const N: usize> {
    words: [Cell<T>; N],
    next: Cell<Option<(usize, T)>>,
}

impl<T: Copy, const N: usize> Commit for MemCell<T, N> {
    fn apply(&self) {
        if let Some((a, v)) = self.next.take() {
            self.words[a % N].set(v)
        }
    }
}

impl<T: Copy + Default + 'static, const N: usize, C: Clock> Default
    for Mem<T, N, C>
{
    fn default() -> Self {
        Mem(
            Rc::new(MemCell {
                words: std::array::from_fn(|_| Cell::new(T::default())),
                next: Cell::new(None),
            }),
            PhantomData,
        )
    }
}

impl<T: Copy + 'static, const N: usize, C: Clock> Mem<T, N, C> {
    /// The write port. Plain and deferred, like a register drive; one
    /// write per step, which is what one port is.
    pub fn write(&self, addr: usize, v: impl Into<T>) {
        self.0.next.set(Some((addr, v.into())));
        commit(self.0.clone());
    }
    /// The read port. Plain, like a wire.
    pub fn read(&self, addr: usize) -> T {
        self.0.words[addr % N].get()
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

/// The prototype's time. One poll of the top unit is one time step, and
/// each clock has an edge at the steps its period and phase say. Every
/// wait is built from [`Tick`]: an edge of a named clock, an operator's
/// cycle in the clock its process is in, and the waits built on those,
/// a channel's `wait` and `until`.
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
        pub fn edge_at(&self, t: u64) -> bool {
            t >= self.phase && (t - self.phase) % self.period == 0
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

/// A wait for a clock edge: of a named clock, or, for an operator's
/// cycle, of whatever clock the process is in.
pub struct Tick {
    clk: Option<clock::Clk>,
}

/// One cycle of the process's clock, unconditionally. Operators and
/// `cycles(n)` use this.
pub fn tick() -> Tick {
    Tick { clk: None }
}

/// The next edge of clock `C`. The wait a process makes once per
/// iteration before it reads state; `C::edge()` is the same thing.
pub fn edge<C: Clock>() -> Tick {
    Tick {
        clk: Some(clk_of::<C>()),
    }
}

/// The next edge of `C` at which `cond` holds. The condition reads
/// state, plainly, and is asked once per edge.
pub async fn until<C: Clock>(mut cond: impl FnMut() -> bool) {
    loop {
        edge::<C>().await;
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
        if !crossed && clk.edge_at(now) {
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

    /// Run the current time step, then advance. Returns whether the
    /// design has finished, which a unit never does.
    pub fn step(&mut self) -> bool {
        if self.done {
            return true;
        }
        let mut cx = Context::from_waker(&self.w);
        self.done = self.f.as_mut().poll(&mut cx).is_ready();
        advance();
        self.done
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

    /// One traced signal: where it is, how wide, and how to read it.
    pub struct Probe {
        pub path: String,
        pub width: usize,
        pub sample: Box<dyn Fn() -> String>,
    }

    thread_local! {
        static PROBES: RefCell<Vec<Probe>> = const { RefCell::new(Vec::new()) };
    }

    /// Register a signal. What the `Traceable` impls call.
    pub fn probe(scope: &Scope, width: usize, sample: Box<dyn Fn() -> String>) {
        PROBES.with(|p| {
            p.borrow_mut().push(Probe {
                path: scope.0.clone(),
                width,
                sample,
            })
        })
    }

    /// Something with signals to register under a scope.
    pub trait Traceable {
        fn trace(&self, scope: &Scope);
    }

    impl<T: Value + 'static, C: Clock> Traceable for Reg<T, C> {
        fn trace(&self, scope: &Scope) {
            let r = self.0.clone();
            probe(scope, T::WIDTH, Box::new(move || r.cur.get().vcd()))
        }
    }
    impl<T: Value + 'static, C: Clock> Traceable for In<T, C> {
        fn trace(&self, scope: &Scope) {
            let c = self.0.clone();
            probe(scope, T::WIDTH, Box::new(move || c.0.get().vcd()))
        }
    }
    impl<T: Value + 'static, C: Clock> Traceable for Out<T, C> {
        fn trace(&self, scope: &Scope) {
            let c = self.0.clone();
            probe(scope, T::WIDTH, Box::new(move || c.0.get().vcd()))
        }
    }
    /// A channel is two signals: the transaction and its valid bit.
    impl<T: Value + super::Transaction + 'static, C: Clock> Traceable for Rx<T, C> {
        fn trace(&self, scope: &Scope) {
            let (c, d) = (self.0.clone(), self.0.clone());
            probe(
                &scope.child("data"),
                T::WIDTH,
                Box::new(move || c.0.get().0.vcd()),
            );
            probe(
                &scope.child("valid"),
                1,
                Box::new(move || d.0.get().1.vcd()),
            );
        }
    }
    impl<T: Value + super::Transaction + 'static, C: Clock> Traceable for Tx<T, C> {
        fn trace(&self, scope: &Scope) {
            let (c, d) = (self.0.clone(), self.0.clone());
            probe(
                &scope.child("data"),
                T::WIDTH,
                Box::new(move || c.0.get().0.vcd()),
            );
            probe(
                &scope.child("valid"),
                1,
                Box::new(move || d.0.get().1.vcd()),
            );
        }
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
    /// step writes its changes. Time is two ticks per step: a clock
    /// rises at the step's edge and falls one tick later, when the
    /// values the step drove have committed.
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
        /// Trace a clock, as a one-bit signal that pulses at each edge.
        pub fn clock<C: Clock>(&mut self) {
            self.clocks.push((C::NAME.to_string(), C::edge_at));
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
            let mut out = self.out;
            clock::TRACER.with(|tr| {
                *tr.borrow_mut() = Some(Box::new(move |t: u64| {
                    let edges: Vec<bool> =
                        clocks.iter().map(|(_, e)| e(t)).collect();
                    if edges.iter().any(|e| *e) {
                        writeln!(out, "#{}", 2 * t).unwrap();
                        for (e, id) in edges.iter().zip(&cids) {
                            if *e {
                                writeln!(out, "1{id}").unwrap();
                            }
                        }
                    }
                    writeln!(out, "#{}", 2 * t + 1).unwrap();
                    for (e, id) in edges.iter().zip(&cids) {
                        if *e {
                            writeln!(out, "0{id}").unwrap();
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

    /// Stop recording, so the sink is dropped and flushed.
    pub fn stop() {
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
}
