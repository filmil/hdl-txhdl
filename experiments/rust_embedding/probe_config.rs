// SPDX-License-Identifier: Apache-2.0
// Probe 6. A configuration is a struct implementing a trait with
// associated types and constants, and the constants are compile-time.
// Against the library's `Config`.
use txhdl::comp::{Config, Unit, Reg, rising, DefaultClock};
use txhdl::types::{Tag, U};

pub trait MacOp: Default { fn mul(&self, a: U<32>, b: U<32>) -> U<64>; }
#[derive(Default)]
pub struct DspSliceMac;
impl MacOp for DspSliceMac {
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

#[derive(Default)]
pub struct Filter<B: Build> { pub mac: B::Mac, pub acc: Reg<U<64>> }
#[derive(Default)]
pub struct Top<B: Build> { pub filter: Filter<B> }

impl<B: Build> Unit<(), ()> for Top<B> {
    async fn run(&mut self, _i: (), _o: ()) {
        let _elastic = <B::Domain as Tag>::HANDSHAKE;
        loop {
            for i in 0..B::TAPS {
                rising::<DefaultClock>().await;
                let acc = self.filter.acc.get();
                self.filter.acc.set(acc.wrapping_add(self.filter.mac.mul(U::new(i as u128), U::new(2))));
            }
        }
    }
}

#[derive(Default)]
pub struct Fpga;
impl Build for Fpga {
    type Mac = DspSliceMac; type Domain = Elastic;
    const TAPS: usize = 16; const CLK_HZ: u64 = 100_000_000;
}
impl Config for Fpga {
    type Top = Top<Fpga>;
    const NAME: &'static str = "fpga";
}

/// A const context accepts nothing but a true constant.
pub const FPGA_TAPS: usize = <Fpga as Build>::TAPS;
pub const TAP_BUFFER: [u64; FPGA_TAPS] = [0; FPGA_TAPS];
