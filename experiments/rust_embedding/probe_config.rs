// Probe 6. A configuration is a struct implementing a trait with
// associated types and constants, and the constants are compile-time.
// Against the library's `Config`.
use txhdl::comp::{Config, Module, Reg};
use txhdl::types::{Tag, U};

pub trait MacOp { fn new() -> Self; fn mul(&self, a: U<32>, b: U<32>) -> U<64>; }
pub struct DspSliceMac;
impl MacOp for DspSliceMac {
    fn new() -> Self { DspSliceMac }
    fn mul(&self, a: U<32>, b: U<32>) -> U<64> { U::new(a.raw() * b.raw()) }
}

pub struct Elastic;
impl Tag for Elastic { const HANDSHAKE: bool = true; const CAPACITY: usize = 8; }

pub trait Build: Config {
    type Mac: MacOp;
    type Domain: Tag;
    const TAPS: usize;
    const CLK_HZ: u64;
}

pub struct Filter<B: Build> { pub mac: B::Mac, pub acc: Reg<U<64>> }
pub struct Top<B: Build> { pub filter: Filter<B> }

impl<B: Build> Module<(), ()> for Top<B> {
    async fn run(&mut self, _i: (), _o: ()) {
        for i in 0..B::TAPS {
            self.filter.acc.set(self.filter.acc.get().wrapping_add(self.filter.mac.mul(U::new(i as u128), U::new(2))));
        }
        let _elastic = <B::Domain as Tag>::HANDSHAKE;
    }
}

pub struct Fpga;
impl Build for Fpga {
    type Mac = DspSliceMac; type Domain = Elastic;
    const TAPS: usize = 16; const CLK_HZ: u64 = 100_000_000;
}
impl Config for Fpga {
    type Top = Top<Fpga>;
    const NAME: &'static str = "fpga";
    fn top() -> Self::Top { Top { filter: Filter { mac: DspSliceMac::new(), acc: Reg::new(U::new(0)) } } }
}

/// A const context accepts nothing but a true constant.
pub const FPGA_TAPS: usize = <Fpga as Build>::TAPS;
pub const TAP_BUFFER: [u64; FPGA_TAPS] = [0; FPGA_TAPS];
