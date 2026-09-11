// Probe 2. The design as instructed, compiled on stable.
//
// Questions this answers:
//   a. Is `async fn` in a trait usable on stable?
//   b. Can inputs and outputs be type parameters, so one struct
//      implements the trait more than once with different shapes?
//   c. Does interface-as-struct plus modport-as-trait hold together?
//   d. Does a non-synthesisable `assert!` sit in the body without fuss?

pub mod comp {
    /// A unit. Inputs and outputs are type parameters rather than
    /// associated types, so one implementation can vary them.
    pub trait Module<In, Out> {
        async fn run(&mut self, inputs: In, outputs: Out);
    }
}

pub mod types {
    /// Marker: this struct is data that moves between units.
    pub trait Transaction {}

    /// A signal carrying a value of type T.
    pub struct Sig<T>(pub core::marker::PhantomData<T>);

    impl<T> Sig<T> {
        pub fn get(&self) -> u128 { 0 }
        pub fn set(&mut self, _v: u128) {}
    }
}

use comp::Module;
use types::{Sig, Transaction};

// --- Transactions: plain structs plus a marker ------------------------

pub struct MacRequest { pub addr: u32, pub count: u8 }
impl Transaction for MacRequest {}

pub struct MacResponse { pub acc: u64 }
impl Transaction for MacResponse {}

// --- Interface: a struct. Modports: traits over it --------------------

/// The interface is the bundle of wires, with no direction on any of them.
pub struct Wishbone {
    pub adr: Sig<u32>,
    pub dat_m2s: Sig<u32>,
    pub dat_s2m: Sig<u32>,
    pub ack: Sig<bool>,
}

/// A modport is a role, stated as a trait: which wires this role drives
/// and which it reads. Direction lives in the trait, not in the struct.
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

// --- A unit, implementing Module twice with different shapes ----------

pub struct MacUnit { pub acc: u64 }

pub struct NarrowIn(pub u32);
pub struct WideIn(pub u64);
pub struct AccOut(pub u64);

impl Module<NarrowIn, AccOut> for MacUnit {
    async fn run(&mut self, inputs: NarrowIn, _outputs: AccOut) {
        // Non-synthesisable, and allowed: it observes and drives nothing.
        assert!(inputs.0 != u32::MAX, "sentinel is not a sample");
        self.acc += inputs.0 as u64;
    }
}

// The same struct, a different input shape. This is what making In and
// Out type parameters rather than associated types buys.
impl Module<WideIn, AccOut> for MacUnit {
    async fn run(&mut self, inputs: WideIn, _outputs: AccOut) {
        self.acc += inputs.0;
    }
}

// --- A unit generic over its own interface role -----------------------

pub struct BusReader<T: Target> { pub port: T }

impl<T: Target> Module<(), AccOut> for BusReader<T> {
    async fn run(&mut self, _inputs: (), _outputs: AccOut) {
        let a = self.port.read_adr();
        self.port.drive_ack(a != 0);
    }
}
