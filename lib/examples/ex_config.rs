// SPDX-License-Identifier: Apache-2.0
//! A design is a program, and a build is one item. `config!` writes the
//! config type and both impls; `Default` builds the top, because a
//! register's default is its reset value and a socket's is its
//! implementation. `main` reaches the top through nothing but the config.
use txhdl::comp::{simulate, Clock, Config, DefaultClock, Module, Reg};
use txhdl::config;
use txhdl::types::U;

pub trait MacOp: Default {
    fn mul(&self, a: U<32>, b: U<32>) -> U<64>;
}
#[derive(Default)]
pub struct DspSliceMac;
#[derive(Default)]
pub struct WallaceTreeMac;
impl MacOp for DspSliceMac {
    fn mul(&self, a: U<32>, b: U<32>) -> U<64> {
        a.mul::<64>(b)
    }
}
impl MacOp for WallaceTreeMac {
    fn mul(&self, a: U<32>, b: U<32>) -> U<64> {
        a.mul::<64>(b)
    }
}

/// Each unit declares the configuration it needs.
pub trait FilterConfig {
    type Mac: MacOp;
    const TAPS: usize;
}
pub trait TopConfig: Config {
    type Filter: FilterConfig;
    const CLK_HZ: u64;
}
pub type FilterOf<T> = <T as TopConfig>::Filter;

#[derive(Default)]
pub struct Filter<FC: FilterConfig> {
    pub mac: FC::Mac,
    pub acc: Reg<U<64>>,
}
#[derive(Default)]
pub struct Top<TC: TopConfig> {
    pub filter: Filter<TC::Filter>,
}

impl<TC: TopConfig> Module<(), ()> for Top<TC> {
    /// A unit runs for as long as the clock does, so this loops. One
    /// wait per cycle, and every tap is computed at that edge: the taps
    /// are parallel hardware, `TAPS` multipliers into one adder chain,
    /// and the accumulator takes their sum at the end of the cycle.
    async fn run(&mut self, _i: (), _o: ()) {
        loop {
            DefaultClock::edge().await;
            let mut acc = self.filter.acc.get();
            for i in 0..<FilterOf<TC> as FilterConfig>::TAPS {
                acc = acc.wrapping_add(self.filter.mac.mul(i.into(), 2.into()));
            }
            self.filter.acc.set(acc);
        }
    }
}

/// Written once, used by any build. A configuration is a unit struct
/// that derives `Default`, because a unit generic over it derives
/// `Default` too and the derive asks that of every parameter.
#[derive(Default)]
pub struct SixteenTaps;
impl FilterConfig for SixteenTaps {
    type Mac = DspSliceMac;
    const TAPS: usize = 16;
}
#[derive(Default)]
pub struct SixtyFourTaps;
impl FilterConfig for SixtyFourTaps {
    type Mac = WallaceTreeMac;
    const TAPS: usize = 64;
}

// A build is one item.
config! { Fpga: TopConfig for Top<Fpga> {
    type Filter = SixteenTaps;
    const CLK_HZ: u64 = 100_000_000;
} }
config! { Asic: TopConfig for Top<Asic> {
    type Filter = SixtyFourTaps;
    const CLK_HZ: u64 = 400_000_000;
} }

/// Simulate one pass over the taps, then observe. `peek` is the
/// testbench's read: it waits for nothing and is not synthesised. The
/// first cycle is spent reaching the first edge, hence the `+ 1`.
fn report<C: TopConfig<Top = Top<C>>>() {
    let taps = <FilterOf<C> as FilterConfig>::TAPS;
    // One cycle is enough: every tap is computed at the first edge.
    let top = simulate::<C>(1);
    println!(
        "{}: {} taps at {} MHz, acc after one cycle = {}",
        C::NAME,
        taps,
        C::CLK_HZ / 1_000_000,
        top.filter.acc.get().raw()
    );
}

fn main() {
    match std::env::args().nth(1).as_deref() {
        Some("asic") => report::<Asic>(),
        _ => report::<Fpga>(),
    }
}
