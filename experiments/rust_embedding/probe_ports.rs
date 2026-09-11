// Probe 14. The relationship between signals, interfaces, roles and
// ports, with direction enforced by the type system rather than by
// convention.
//
// The claim being tested: direction is not a property of a signal. The
// same wire is driven at one end and read at the other, so direction
// belongs to the *view*, not to the thing viewed. That is exactly the
// mistake LHDL made by giving `interface` both jobs.
//
//   Signal<T>      one wire. No direction, no handshake.
//   Chan<T>        one channel: a transaction plus handshaking.
//   Interface      a struct of those. Still no direction.
//   Role           a view: per member, which way this end faces.
//   Port           an interface seen through a role. What `run` is given.

use core::cell::Cell;
use core::marker::PhantomData;

pub mod types {
    pub trait Transaction: Copy + Default {}
}

pub mod comp {
    use super::*;

    /// One wire. It knows how to hold a value and nothing about who
    /// drives it.
    pub struct Signal<T: Copy + Default> {
        v: Cell<T>,
    }

    impl<T: Copy + Default> Signal<T> {
        pub fn new() -> Self { Signal { v: Cell::new(T::default()) } }
        pub(crate) fn read(&self) -> T { self.v.get() }
        pub(crate) fn drive(&self, x: T) { self.v.set(x) }
    }

    impl<T: Copy + Default> Default for Signal<T> {
        fn default() -> Self { Self::new() }
    }

    /// One channel: a transaction plus the handshake the compiler
    /// supplies.
    pub struct Chan<T: types::Transaction> {
        v: Cell<T>,
        valid: Cell<bool>,
    }

    impl<T: types::Transaction> Chan<T> {
        pub fn new() -> Self { Chan { v: Cell::new(T::default()), valid: Cell::new(false) } }
        pub(crate) fn put(&self, x: T) { self.v.set(x); self.valid.set(true) }
        pub(crate) fn take(&self) -> Option<T> {
            if self.valid.get() { self.valid.set(false); Some(self.v.get()) } else { None }
        }
    }

    impl<T: types::Transaction> Default for Chan<T> {
        fn default() -> Self { Self::new() }
    }

    /// A read-only view of a wire. There is no `set` on it, so a unit
    /// holding one cannot drive it. That is the whole enforcement.
    pub struct In<'a, T: Copy + Default>(pub(crate) &'a Signal<T>);

    /// A drive-only view.
    pub struct Out<'a, T: Copy + Default>(pub(crate) &'a Signal<T>);

    impl<'a, T: Copy + Default> In<'a, T> {
        pub fn get(&self) -> T { self.0.read() }
    }

    impl<'a, T: Copy + Default> Out<'a, T> {
        pub fn set(&self, x: T) { self.0.drive(x) }
    }

    /// The same split for a channel.
    pub struct Rx<'a, T: types::Transaction>(pub(crate) &'a Chan<T>);
    pub struct Tx<'a, T: types::Transaction>(pub(crate) &'a Chan<T>);

    impl<'a, T: types::Transaction> Rx<'a, T> {
        pub fn recv(&self) -> Option<T> { self.0.take() }
    }

    impl<'a, T: types::Transaction> Tx<'a, T> {
        pub fn send(&self, x: T) { self.0.put(x) }
    }

    /// Marker: a struct that is an interface.
    pub trait Interface {}

    /// Marker: a role an interface may be viewed in.
    pub trait Role {}

    /// An interface seen through a role. `View` is what a unit is handed.
    pub trait Viewed<'a, R: Role>: Interface {
        type View;
        fn view(&'a self) -> Self::View;
    }

    /// What a unit declares it needs. A port is a role applied to an
    /// interface, and nothing more.
    pub struct Port<I, R: Role>(PhantomData<(I, R)>);

    #[allow(async_fn_in_trait)]
    pub trait Module<In, Out> {
        async fn run(&mut self, inputs: In, outputs: Out);
    }
}

use comp::*;
use types::Transaction;

#[derive(Clone, Copy, Default)]
pub struct Beat { pub data: u32 }
impl Transaction for Beat {}

// --- an interface: signals and channels, no direction anywhere -------

#[derive(Default)]
pub struct Wishbone {
    pub adr: Signal<u32>,
    pub ack: Signal<bool>,
    pub dat: Chan<Beat>,
}

impl Interface for Wishbone {}

// --- two roles, as types --------------------------------------------

pub struct Initiator;
pub struct Target;
impl Role for Initiator {}
impl Role for Target {}

// --- the views each role gets ---------------------------------------
//
// A derive would generate these. Written out here so the probe shows
// what is generated rather than hiding it.

pub struct InitiatorView<'a> {
    pub adr: Out<'a, u32>,
    pub ack: In<'a, bool>,
    pub dat: Tx<'a, Beat>,
}

pub struct TargetView<'a> {
    pub adr: In<'a, u32>,
    pub ack: Out<'a, bool>,
    pub dat: Rx<'a, Beat>,
}

impl<'a> Viewed<'a, Initiator> for Wishbone {
    type View = InitiatorView<'a>;
    fn view(&'a self) -> InitiatorView<'a> {
        InitiatorView { adr: Out(&self.adr), ack: In(&self.ack), dat: Tx(&self.dat) }
    }
}

impl<'a> Viewed<'a, Target> for Wishbone {
    type View = TargetView<'a>;
    fn view(&'a self) -> TargetView<'a> {
        TargetView { adr: In(&self.adr), ack: Out(&self.ack), dat: Rx(&self.dat) }
    }
}

// --- two units, each seeing the same wires the other way round -------

pub struct Cpu { pub next: Cell<u32> }
pub struct Ram { pub hits: Cell<u32> }

impl<'a> Module<InitiatorView<'a>, ()> for Cpu {
    async fn run(&mut self, p: InitiatorView<'a>, _o: ()) {
        p.adr.set(self.next.get());      // allowed: Out
        let acked = p.ack.get();         // allowed: In
        p.dat.send(Beat { data: 7 });    // allowed: Tx
        if acked { self.next.set(self.next.get() + 1) }
    }
}

impl<'a> Module<TargetView<'a>, ()> for Ram {
    async fn run(&mut self, p: TargetView<'a>, _o: ()) {
        let a = p.adr.get();             // allowed: In
        p.ack.set(a != 0);               // allowed: Out
        if p.dat.recv().is_some() { self.hits.set(self.hits.get() + 1) }
    }
}

// --- the parent owns the interface and hands out both views ----------

pub struct Top { pub cpu: Cpu, pub ram: Ram, pub bus: Wishbone }

impl Top {
    pub fn new() -> Self {
        Top { cpu: Cpu { next: Cell::new(0) }, ram: Ram { hits: Cell::new(0) },
              bus: Wishbone::default() }
    }
}

impl Module<(), ()> for Top {
    async fn run(&mut self, _i: (), _o: ()) {
        let m: InitiatorView<'_> = Viewed::<Initiator>::view(&self.bus);
        let t: TargetView<'_> = Viewed::<Target>::view(&self.bus);
        self.cpu.run(m, ()).await;
        self.ram.run(t, ()).await;
    }
}

pub fn build() -> Top { Top::new() }
