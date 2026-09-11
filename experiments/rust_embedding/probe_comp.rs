// Probe 3. The `crate::comp` vocabulary, compiled on stable.
//
// comp::Module  a unit, with inputs and outputs as type parameters
// comp::Bus     a bundle of channels
// comp::Signal  one wire
// comp::Port    a unit's attachment to a bus, in a role
// types::Transaction  data that moves over a channel
//
// `async fn` in a trait is stable, and warns that auto trait bounds such
// as `Send` cannot be stated. Elaboration here is single threaded, so the
// bound is not wanted and the lint is allowed once, at the trait.

pub mod types {
    /// Marker: a struct that moves between units.
    pub trait Transaction {}

    /// A synchronisation domain and its policy. A tag is a type, not an
    /// attribute, which is what lets a unit be generic over it. It also
    /// sidesteps the fact that an attribute on a block is not stable.
    pub trait Tag {
        /// Inject ready and valid across the domain.
        const HANDSHAKE: bool = false;
        /// Track this many transactions in flight, 0 for unlimited.
        const CAPACITY: usize = 0;
    }
}

pub mod comp {
    use crate::types::Transaction;
    use core::marker::PhantomData;

    /// One wire, or one bundle of wires, carrying a value of type T.
    pub struct Signal<T> {
        _t: PhantomData<T>,
    }

    impl<T> Signal<T> {
        pub const fn new() -> Self { Self { _t: PhantomData } }
        pub fn get(&self) -> u128 { 0 }
        pub fn set(&mut self, _v: u128) {}
    }

    impl<T> Default for Signal<T> {
        fn default() -> Self { Self::new() }
    }

    /// A channel: one transaction type, with handshaking the compiler
    /// supplies. Declared in a bus, never by hand.
    pub struct Chan<T: Transaction> {
        _t: PhantomData<T>,
    }

    impl<T: Transaction> Chan<T> {
        pub const fn new() -> Self { Self { _t: PhantomData } }
    }

    impl<T: Transaction> Default for Chan<T> {
        fn default() -> Self { Self::new() }
    }

    /// Marker: a struct that is a bus.
    pub trait Bus {}

    /// Marker: a role a bus can be attached in. Modports are traits; this
    /// is the type-level tag that says which one a port takes.
    pub trait Role {}

    /// A unit's attachment to a bus, in one role.
    pub struct Port<B: Bus, R: Role> {
        pub bus: B,
        _r: PhantomData<R>,
    }

    impl<B: Bus, R: Role> Port<B, R> {
        pub fn new(bus: B) -> Self { Self { bus, _r: PhantomData } }
    }

    /// A unit. Inputs and outputs are type parameters rather than
    /// associated types, so one implementation can vary them.
    #[allow(async_fn_in_trait)]
    pub trait Module<In, Out> {
        async fn run(&mut self, inputs: In, outputs: Out);
    }
}

// ---------------------------------------------------------------------

use comp::{Bus, Chan, Module, Port, Role, Signal};
use types::{Tag, Transaction};

/// A tag is a type. Its policy is associated constants, so the values are
/// known at compile time and a unit can be generic over the whole domain.
pub struct MemFetch;
impl Tag for MemFetch {
    const HANDSHAKE: bool = true;
    const CAPACITY: usize = 8;
}

pub struct Combinational;
impl Tag for Combinational {}

pub struct MemRead { pub addr: u32 }
impl Transaction for MemRead {}

pub struct MemData { pub data: u32 }
impl Transaction for MemData {}

/// A bus is a struct of channels plus the marker.
#[derive(Default)]
pub struct MemBus {
    pub request: Chan<MemRead>,
    pub response: Chan<MemData>,
}
impl Bus for MemBus {}

/// Roles, as type-level tags.
pub struct Master;
pub struct Slave;
impl Role for Master {}
impl Role for Slave {}

/// An interface is a struct of wires with no direction on any of them.
#[derive(Default)]
pub struct Wishbone {
    pub adr: Signal<u32>,
    pub ack: Signal<bool>,
}
impl Bus for Wishbone {}

/// A modport is a role stated as a trait. Direction lives here, not in
/// the struct, which is what stops one item doing two jobs.
pub trait Initiator {
    fn drive_adr(&mut self, v: u32);
    fn read_ack(&self) -> bool;
}

pub trait Target {
    fn read_adr(&self) -> u32;
    fn drive_ack(&mut self, v: bool);
}

impl Initiator for Wishbone {
    fn drive_adr(&mut self, v: u32) { self.adr.set(v as u128); }
    fn read_ack(&self) -> bool { self.ack.get() != 0 }
}

impl Target for Wishbone {
    fn read_adr(&self) -> u32 { self.adr.get() as u32 }
    fn drive_ack(&mut self, v: bool) { self.ack.set(v as u128); }
}

// ---------------------------------------------------------------------

pub struct MacRequest { pub addr: u32, pub count: u8 }
impl Transaction for MacRequest {}

pub struct MacResponse { pub acc: u64 }
impl Transaction for MacResponse {}

/// A unit. It holds its ports and its state, and names the domain it runs
/// in as a type parameter rather than as an attribute.
pub struct MacUnit<T: Tag> {
    pub mem: Port<MemBus, Master>,
    pub acc: u64,
    _tag: core::marker::PhantomData<T>,
}

/// Timeless: not async, so no `.await`, so no cycle count can be written.
fn mul(a: u32, b: u32) -> u64 { a as u64 * b as u64 }

impl<T: Tag> Module<MacRequest, MacResponse> for MacUnit<T> {
    async fn run(&mut self, inputs: MacRequest, _outputs: MacResponse) {
        // Non-synthesisable and allowed: it reads and drives nothing.
        assert!(inputs.count > 0, "empty accumulate has no defined result");
        // The tag's policy is a compile-time constant, readable here.
        let _elastic = T::HANDSHAKE;
        self.acc = 0;
        for _ in 0..inputs.count {
            self.acc += mul(inputs.addr, 2);
        }
    }
}

/// A unit generic over the role it attaches in, rather than over a bus.
pub struct BusReader<T: Target> { pub port: T }

impl<T: Target> Module<(), ()> for BusReader<T> {
    async fn run(&mut self, _inputs: (), _outputs: ()) {
        let a = self.port.read_adr();
        self.port.drive_ack(a != 0);
    }
}

/// Construction, to prove the pieces fit together. The domain is chosen
/// here, the way a configuration would choose it.
pub fn build() -> MacUnit<MemFetch> {
    MacUnit {
        mem: Port::new(MemBus::default()),
        acc: 0,
        _tag: core::marker::PhantomData,
    }
}

/// The same unit in a different domain: one type alias, no edit to the
/// unit. This is the configuration facet.
pub type Elastic = MacUnit<MemFetch>;
pub type Raw = MacUnit<Combinational>;
