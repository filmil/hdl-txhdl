// SPDX-License-Identifier: Apache-2.0
//! A design is a program. A configuration is a tree: each unit declares
//! what it needs, and a parent's names its children's, so two children
//! may share a knob name and a child's configuration is reused across
//! builds. `main` reaches the top through nothing but the config.
use txhdl::comp::{elaborate, Config, Module, Reg};
use txhdl::types::U;

pub trait MacOp { fn new() -> Self; fn mul(&self, a: U<32>, b: U<32>) -> U<64>; }
pub struct DspSliceMac;
pub struct WallaceTreeMac;
impl MacOp for DspSliceMac {
    fn new() -> Self { DspSliceMac }
    fn mul(&self, a: U<32>, b: U<32>) -> U<64> { U::new(a.raw() * b.raw()) }
}
impl MacOp for WallaceTreeMac {
    fn new() -> Self { WallaceTreeMac }
    fn mul(&self, a: U<32>, b: U<32>) -> U<64> { U::new(a.raw() * b.raw()) }
}

/// Each unit declares the configuration it needs.
pub trait FilterConfig { type Mac: MacOp; const TAPS: usize; }
pub trait TopConfig: Config { type Filter: FilterConfig; const CLK_HZ: u64; }

pub struct Filter<FC: FilterConfig> { pub mac: FC::Mac, pub acc: Reg<U<64>> }
pub struct Top<TC: TopConfig> { pub filter: Filter<TC::Filter> }

/// One alias per level of the configuration tree, so reaching a child's
/// constant is one qualification and not two.
pub type FilterOf<T> = <T as TopConfig>::Filter;

impl<TC: TopConfig> Module<(), ()> for Top<TC> {
    async fn run(&mut self, _i: (), _o: ()) {
        for i in 0..<FilterOf<TC> as FilterConfig>::TAPS {
            let p = self.filter.mac.mul(U::new(i as u128), U::new(2));
            self.filter.acc.set(self.filter.acc.get().wrapping_add(p));
        }
    }
}

/// Written once, used by both builds.
pub struct SixteenTaps;
impl FilterConfig for SixteenTaps { type Mac = DspSliceMac; const TAPS: usize = 16; }
pub struct SixtyFourTaps;
impl FilterConfig for SixtyFourTaps { type Mac = WallaceTreeMac; const TAPS: usize = 64; }

pub struct Fpga;
impl TopConfig for Fpga { type Filter = SixteenTaps; const CLK_HZ: u64 = 100_000_000; }
impl Config for Fpga {
    type Top = Top<Fpga>;
    const NAME: &'static str = "fpga";
    fn top() -> Self::Top {
        Top { filter: Filter { mac: DspSliceMac::new(), acc: Reg::new(U::new(0)) } }
    }
}

pub struct Asic;
impl TopConfig for Asic { type Filter = SixtyFourTaps; const CLK_HZ: u64 = 400_000_000; }
impl Config for Asic {
    type Top = Top<Asic>;
    const NAME: &'static str = "asic";
    fn top() -> Self::Top {
        Top { filter: Filter { mac: WallaceTreeMac::new(), acc: Reg::new(U::new(0)) } }
    }
}

/// `elaborate` returns whatever `C::Top` is. Naming it as `Top<C>` here
/// is what lets the report read a field of it.
fn report<C: TopConfig<Top = Top<C>>>() {
    let top = elaborate::<C>();
    println!("{}: {} taps at {} MHz, acc = {}",
        C::NAME, <FilterOf<C> as FilterConfig>::TAPS, C::CLK_HZ / 1_000_000,
        top.filter.acc.get().raw());
}

fn main() {
    match std::env::args().nth(1).as_deref() {
        Some("asic") => report::<Asic>(),
        _ => report::<Fpga>(),
    }
}
