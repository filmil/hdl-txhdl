// Probe 6. `config` as a struct.
//
// A config gathers every choice a build makes: which implementation fills
// each socket, what each generic parameter is, and what the physical
// numbers are. A trait with associated types and associated constants
// holds all three, and a unit stays generic over the whole config rather
// than over one parameter per choice.
//
// The question is whether a path-shaped binding, `bind top.filter.mac =>
// Impl`, survives. It does, and it stops being a path: a nested unit is
// generic over the same config, so the choice propagates without anyone
// naming a hierarchy.

pub mod types {
    pub trait Tag {
        const HANDSHAKE: bool = false;
        const CAPACITY: usize = 0;
    }
}

pub mod comp {
    #[allow(async_fn_in_trait)]
    pub trait Module<In, Out> {
        async fn run(&mut self, inputs: In, outputs: Out);
    }

    /// A named build. Every choice the design defers lives here.
    pub trait Config {
        /// Sockets, as associated types.
        type Mac: super::MacOp;
        /// Domains, as associated types.
        type Clock: super::types::Tag;
        /// Generic parameters, as associated constants.
        const TAPS: usize;
        /// Physical numbers the design facet must not name.
        const CLK_HZ: u64;
    }
}

use comp::{Config, Module};
use types::Tag;

// --- a contract and two implementations of it ------------------------

pub trait MacOp {
    fn mul(&self, a: u32, b: u32) -> u64;
    fn new() -> Self;
}

pub struct DspSliceMac;
impl MacOp for DspSliceMac {
    fn mul(&self, a: u32, b: u32) -> u64 { a as u64 * b as u64 }
    fn new() -> Self { DspSliceMac }
}

pub struct WallaceTreeMac;
impl MacOp for WallaceTreeMac {
    fn mul(&self, a: u32, b: u32) -> u64 { a as u64 * b as u64 }
    fn new() -> Self { WallaceTreeMac }
}

pub struct Elastic;
impl Tag for Elastic {
    const HANDSHAKE: bool = true;
    const CAPACITY: usize = 8;
}

pub struct Raw;
impl Tag for Raw {}

// --- two builds ------------------------------------------------------

pub struct Fpga;
impl Config for Fpga {
    type Mac = DspSliceMac;
    type Clock = Elastic;
    const TAPS: usize = 16;
    const CLK_HZ: u64 = 100_000_000;
}

pub struct Asic;
impl Config for Asic {
    type Mac = WallaceTreeMac;
    type Clock = Raw;
    const TAPS: usize = 64;
    const CLK_HZ: u64 = 400_000_000;
}

// --- the design, generic over the whole build ------------------------

/// A nested unit. It does not name a path and does not name an
/// implementation. It is generic over the config, so the choice reaches
/// it without `bind top.filter.mac` ever being written.
pub struct Filter<C: Config> {
    pub mac: C::Mac,
    pub acc: u64,
}

impl<C: Config> Filter<C> {
    pub fn new() -> Self { Filter { mac: C::Mac::new(), acc: 0 } }
}

pub struct Top<C: Config> {
    pub filter: Filter<C>,
}

impl<C: Config> Top<C> {
    pub fn new() -> Self { Top { filter: Filter::<C>::new() } }
}

pub struct Ins { pub a: u32, pub b: u32 }
pub struct Outs;

impl<C: Config> Module<Ins, Outs> for Top<C> {
    async fn run(&mut self, inputs: Ins, _outputs: Outs) {
        // Associated constants are compile-time values, usable as bounds
        // and as loop counts.
        for _ in 0..C::TAPS {
            self.filter.acc += self.filter.mac.mul(inputs.a, inputs.b);
        }
        // The domain's policy, reached through the config.
        let _elastic = <C::Clock as Tag>::HANDSHAKE;
        let _hz = C::CLK_HZ;
    }
}

// --- the configuration facet, entire ---------------------------------

pub type FpgaBuild = Top<Fpga>;
pub type AsicBuild = Top<Asic>;

pub fn build_fpga() -> FpgaBuild { Top::<Fpga>::new() }
pub fn build_asic() -> AsicBuild { Top::<Asic>::new() }

/// A const context, to prove the numbers really are compile time.
pub const FPGA_TAPS: usize = <Fpga as Config>::TAPS;
pub const ASIC_TAPS: usize = <Asic as Config>::TAPS;
pub const TAP_BUFFER: [u64; FPGA_TAPS] = [0; FPGA_TAPS];
