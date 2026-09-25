// SPDX-License-Identifier: Apache-2.0
//! An array of registers and a loop over it, in a lowered body (issue
//! 594).
//!
//! `Tally<N>` counts the hits on each of `N` input lines, a register
//! per line. The registers are one field, `counts: Regs<U<8>, N>`, and
//! the body reads and drives `self.counts[i]` in `for i in 0..N`, the
//! same loop that goes over the array of ports `hits`. A count stops at
//! 255 rather than wrapping. The outputs are the sum of the counts and
//! whether any of them is full. The
//! netlist of `Tally<3>` has the registers `counts_0` to `counts_2`,
//! and is checked against the run under nvc and Verilator.
use txhdl::comp::trace::{stop, Wave};
use txhdl::comp::{signal, Clock, DefaultClock, In, Out, Regs, Running, Unit};
use txhdl::types::{Bit, U};
use txhdl::{lower, with, Trace};

// begin{unit}
/// A count of hits per input line.
#[derive(Trace, Default)]
pub struct Tally<const N: usize> {
    /// One count per line, `counts_0` to `counts_{N-1}` in the netlist.
    pub counts: Regs<U<8>, N>,
}

// The lowering reads a loop over an array as `hits[i]`, so the index is
// what it is written with, and Clippy would rather it were an iterator.
#[allow(clippy::needless_range_loop)]
#[lower]
impl<const N: usize> Unit for Tally<N> {
    async fn run(
        &mut self,
        hits: [In<Bit>; N],
        (total, full): (Out<U<8>>, Out<Bit>),
    ) {
        loop {
            DefaultClock::rising().await;
            let mut sum = U::<8>::from(0u8);
            let mut any = Bit::Zero;
            for i in 0..N {
                let c = self.counts[i].get();
                let top = c == 255;
                sum = sum + c;
                any = any | top;
                let up = hits[i].get() & !top;
                with!(self <= { up ? counts[i]: c + 1 });
            }
            total.set(sum);
            full.set(any);
        }
    }
}
// end{unit}

fn main() {
    let (h0_out, h0) = signal::<Bit, DefaultClock>();
    let (h1_out, h1) = signal::<Bit, DefaultClock>();
    let (h2_out, h2) = signal::<Bit, DefaultClock>();
    let (total_out, total) = signal::<U<8>, DefaultClock>();
    let (full_out, full) = signal::<Bit, DefaultClock>();
    let mut unit = Tally::<3>::default();
    let hits = [h0, h1, h2];
    if let Some(mut w) = Wave::from_env() {
        w.clock::<DefaultClock>();
        w.add("hits_0", &hits[0]);
        w.add("hits_1", &hits[1]);
        w.add("hits_2", &hits[2]);
        w.add("tallies", &unit);
        w.add("total", &total);
        w.add("full", &full);
        w.start();
    }
    let lines = [h0_out, h1_out, h2_out];
    let mut sim = Running::new(unit.run(hits, (total_out, full_out)));
    // Line 0 is hit every cycle, line 1 every other, line 2 every
    // third.
    let mut sums: Vec<u128> = Vec::new();
    for cycle in 0..20u32 {
        for (i, line) in lines.iter().enumerate() {
            line.set(Bit::from(cycle % (i as u32 + 1) == 0));
        }
        sim.cycle();
        sums.push(total.get().raw());
    }
    println!("totals: {sums:?}");
    assert_eq!(full.get(), Bit::Zero, "no count reached 255");
    stop();
    let net = Tally::<3>::lowered("tallies");
    let regs: Vec<&str> = net
        .fields
        .iter()
        .filter(|f| f.1 == Some(txhdl::comp::trace::Kind::Reg))
        .map(|f| f.0)
        .collect();
    println!("registers: {regs:?}");
    txhdl::netlist::write_vhdl_from_env(&net);
    print!("\n{}", net.verilog());
}
