// SPDX-License-Identifier: Apache-2.0
//! An address map as a trait constant (issue 593).
//!
//! A unit over `N` peripherals wants `N` address ranges, and stable
//! Rust has no array const parameter to carry them. So the map is a
//! type: `Three` below implements `AddrMap<3>` with its three ranges as
//! an associated constant, and `Decode<3, Three>` is the decoder over
//! them. The unit reads `M::RANGES[i]` in a loop over `0..N`; the
//! lowering unrolls the loop when `lowered` runs and works each base
//! and mask out as a number of the netlist, as it does a const
//! parameter, so the netlist decodes the ranges the map states. The
//! array's length is checked against `N` where the map is written: a
//! map of two ranges given to a decoder of three does not compile.
//!
//! The decoder takes an address a cycle and says, one bit per range,
//! which range holds it, and whether none does. The run walks a few
//! addresses in and out of the ranges and asserts what it sees; the
//! netlist is checked against the run under nvc and Verilator.
use std::marker::PhantomData;

use txhdl::comp::trace::{stop, Wave};
use txhdl::comp::{
    mux, now, signal, Clock, DefaultClock, In, Out, Reg, Running, Unit,
};
use txhdl::map::AddrMap;
use txhdl::types::{Bit, U};
use txhdl::{lower, with, Trace};

// begin{map}
/// A design's map: a timer at 0x1000, a serial port at 0x2000, and a
/// finer range for a flag register at 0x3000, each a base and the mask
/// that picks the range out.
pub struct Three;

impl AddrMap<3> for Three {
    const RANGES: [(usize, usize); 3] =
        [(0x1000, 0xf000), (0x2000, 0xf000), (0x3000, 0xff00)];
}
// end{map}

// begin{unit}
/// The decoder over `N` ranges, generic over the map that states them.
#[derive(Trace)]
pub struct Decode<const N: usize, M: AddrMap<N>> {
    /// The map, which holds no signal: a marker the netlist ignores.
    pub map: PhantomData<M>,
    /// Addresses that hit some range.
    pub hits: Reg<U<16>>,
}

impl<const N: usize, M: AddrMap<N>> Default for Decode<N, M> {
    fn default() -> Self {
        Decode {
            map: PhantomData,
            hits: Reg::default(),
        }
    }
}

// The lowering reads the loop as `M::RANGES[i]`, so the index is what
// it is written with, and Clippy would rather it were an iterator.
#[allow(clippy::needless_range_loop)]
#[lower]
impl<const N: usize, M: AddrMap<N>> Unit for Decode<N, M> {
    async fn run(
        &mut self,
        addr: In<U<32>>,
        (sel, none): (Out<U<N>>, Out<Bit>),
    ) {
        loop {
            DefaultClock::rising().await;
            let a = addr.get();
            let mut picked = U::<N>::from(0u8);
            let mut any = Bit::Zero;
            for i in 0..N {
                // An address is in a range when the mask keeps its
                // base; the first range that matches is the one.
                let hit = Bit::from(
                    (a.raw() as usize & M::RANGES[i].1) == M::RANGES[i].0,
                ) & !any;
                picked = mux(hit, U::<N>::from(1u32 << i), picked);
                any = any | hit;
            }
            with!(self <= { any ? hits: self.hits.get() + 1 });
            sel.set(picked);
            none.set(!any);
        }
    }
}
// end{unit}

fn main() {
    let (addr_out, addr) = signal::<U<32>, DefaultClock>();
    let (sel_out, sel) = signal::<U<3>, DefaultClock>();
    let (none_out, none) = signal::<Bit, DefaultClock>();
    let mut decode = Decode::<3, Three>::default();
    let hits = decode.hits;
    if let Some(mut w) = Wave::from_env() {
        w.clock::<DefaultClock>();
        w.add("addr", &addr);
        w.add("decode", &decode);
        w.add("sel", &sel);
        w.add("none", &none);
        w.start();
    }
    let mut sim = Running::new(decode.run(addr, (sel_out, none_out)));
    // An address in each range, one between them, one past them, and
    // one in the finer range's neighbourhood that its mask leaves out.
    let walk: [(u32, u32); 7] = [
        (0x1004, 0b001),
        (0x1ffc, 0b001),
        (0x2000, 0b010),
        (0x0800, 0b000),
        (0x3040, 0b100),
        (0x3140, 0b000),
        (0x4000, 0b000),
    ];
    for &(a, want) in &walk {
        addr_out.set(U::<32>::from(a));
        sim.cycle();
        println!(
            "t={:>2} addr {:#07x} sel {:03b} none {}",
            now(),
            a,
            sel.get().raw(),
            none.get().to_bool() as u8
        );
        assert_eq!(sel.get().raw(), want as u128, "the range of {a:#x}");
        assert_eq!(none.get().to_bool(), want == 0, "no range for {a:#x}");
    }
    stop();
    println!(
        "{} of {} addresses hit a range",
        hits.get().raw(),
        walk.len()
    );
    let net = Decode::<3, Three>::lowered("decode");
    txhdl::netlist::write_vhdl_from_env(&net);
    print!("\n{}", net.verilog());
}
