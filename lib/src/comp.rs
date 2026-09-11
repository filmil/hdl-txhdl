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
    pub fn step(&self) {
        self.to.set(self.from.get())
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
pub struct Reg<T: Copy, C: Clock = DefaultClock>(Cell<T>, PhantomData<C>);

impl<T: Copy + Default, C: Clock> Default for Reg<T, C> {
    fn default() -> Self {
        Reg(Cell::new(T::default()), PhantomData)
    }
}

impl<T: Copy, C: Clock> Reg<T, C> {
    pub fn new(v: impl Into<T>) -> Self {
        Reg(Cell::new(v.into()), PhantomData)
    }

    /// Read the register. Awaiting this is waiting for the clock edge
    /// that latched the value. In the prototype it yields to the
    /// executor once, so one poll of a unit is one clock cycle, and a
    /// process that loops advances one iteration per cycle.
    pub async fn get(&self) -> T {
        edge::<C>(self as *const Self as usize).await;
        self.0.get()
    }

    /// Observe the register without waiting. Not synthesisable, and
    /// allowed for the reason `assert!` is: it reads and drives nothing.
    /// For testbenches and reports, never for a design.
    pub fn peek(&self) -> T {
        self.0.get()
    }

    /// Drive the next value. A drive is plain: nothing is waited for.
    pub fn set(&self, v: impl Into<T>) {
        self.0.set(v.into())
    }

    /// A predicated drive: a multiplexer on the enable, not a branch.
    pub fn set_if(&self, pred: Bit, v: impl Into<T>) {
        if pred.to_bool() {
            self.0.set(v.into())
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
    [Cell<T>; N],
    PhantomData<C>,
);

impl<T: Copy + Default, const N: usize, C: Clock> Default for Mem<T, N, C> {
    fn default() -> Self {
        Mem(
            std::array::from_fn(|_| Cell::new(T::default())),
            PhantomData,
        )
    }
}

impl<T: Copy, const N: usize, C: Clock> Mem<T, N, C> {
    /// The write port. Plain, like a register drive.
    pub fn write(&self, addr: usize, v: impl Into<T>) {
        self.0[addr % N].set(v.into())
    }
    /// The read port. Plain, like a wire.
    pub fn read(&self, addr: usize) -> T {
        self.0[addr % N].get()
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
/// step and completes when all have, with their outputs as a tuple, so
/// two register reads inside it are one wait for one edge, and the
/// executor knows it: a read polled inside `parallel!` after another
/// read has crossed the edge is free. The same two reads written one
/// after the other are two waits, because every `.await` is a cycle
/// boundary and this is the one way to say that two are not.
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
                clock::PARALLEL.with(|p| p.set(p.get() + 1));
                let mut all = true;
                $( all &= unsafe { Pin::new_unchecked(&mut t.$i) }.poll(cx); )+
                clock::PARALLEL.with(|p| p.set(p.get() - 1));
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
/// wait is built from [`Tick`]: a register read waits for the edge of
/// the register's clock, an operator's cycle waits for the next edge of
/// the clock its process is in.
///
/// Every `.await` is a cycle boundary. A register read waits for the
/// next edge of the register's clock, and so does a second read written
/// after it. Reads that share one edge are written inside `parallel!`,
/// and that is the one case the executor treats specially: a read polled
/// inside a `parallel!` whose process has already crossed the edge at
/// this step is free, so the whole group is one wait. Processes are
/// told apart by their waker, which [`join2`] and [`join_all`] give
/// each child fresh, so the rule holds per process and not per unit. A
/// process is in the domain of the last edge it crossed; the executor
/// does not check that it stays there, and a register of another domain
/// read without a [`Crossing`] is a hazard the prototype runs rather
/// than refuses.
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
        /// How many `parallel!` groups are being polled right now.
        pub static PARALLEL: Cell<u32> = const { Cell::new(0) };
        pub static PROCS: RefCell<HashMap<usize, Proc>> =
            RefCell::new(HashMap::new());
        /// Per process and register: the last time step it was read at.
        pub static READ: RefCell<HashMap<(usize, usize), u64>> =
            RefCell::new(HashMap::new());
    }
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

/// A wait for a clock edge. With a key, the read of one register, keyed
/// by the register and waiting on its clock; without one, an operator's
/// cycle in whatever clock the process is in.
pub struct Tick {
    key: Option<usize>,
    clk: Option<clock::Clk>,
}

/// One cycle of the process's clock, unconditionally. Operators and
/// `cycles(n)` use this.
pub fn tick() -> Tick {
    Tick {
        key: None,
        clk: None,
    }
}

/// The edge a register read waits for. Free only inside `parallel!`,
/// once another read in the group has crossed the edge of `C` at this
/// step, and only once per register.
fn edge<C: Clock>(key: usize) -> Tick {
    Tick {
        key: Some(key),
        clk: Some(clk_of::<C>()),
    }
}

impl Future for Tick {
    type Output = ();
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        let p = cx.waker().data() as usize;
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
        if let Some(k) = self.key {
            let unread =
                clock::READ.with(|r| r.borrow().get(&(p, k)) != Some(&now));
            if grouped && crossed && unread {
                clock::READ.with(|r| r.borrow_mut().insert((p, k), now));
                return Poll::Ready(());
            }
        }
        if !clk.edge_at(now) || crossed {
            return Poll::Pending;
        }
        clock::PROCS
            .with(|m| m.borrow_mut().insert(p, clock::Proc { clk, edge: now }));
        if let Some(k) = self.key {
            clock::READ.with(|r| r.borrow_mut().insert((p, k), now));
        }
        Poll::Ready(())
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

/// Advance time by one step.
fn advance() {
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
