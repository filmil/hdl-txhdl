// SPDX-License-Identifier: Apache-2.0
//! A sorted list in registers, each place taking its neighbour's byte
//! as a byte is put in below it (issue 1851).
//!
//! `Sorted<N>` keeps the `N` largest bytes it is given, largest first,
//! in `l: Regs<U<8>, N>`, which start at zero. A byte in goes after
//! every byte not below it, at `p`; the places before `p`
//! keep theirs, place `p` takes the new byte, and each place after it
//! takes the byte of the place before it, `self.l[j - 1]`, the last
//! byte falling off. The loop over the places starts at 1, since place
//! 0 has no place before it: its index would be below zero, which the
//! lowering refuses when it unrolls the loop. The netlist of
//! `Sorted<4>` reads `l_0` into `l_1`, `l_1` into `l_2` and `l_2` into
//! `l_3`, and is checked against the run under nvc and Verilator.
use txhdl::comp::trace::{stop, Wave};
use txhdl::comp::{
    mux, signal, Clock, DefaultClock, In, Out, Regs, Running, Unit,
};
use txhdl::types::{Bit, U};
use txhdl::{lower, with, Trace};

// begin{unit}
/// The `N` largest bytes so far, largest first.
#[derive(Trace, Default)]
pub struct Sorted<const N: usize> {
    /// The places, `l_0` to `l_{N-1}` in the netlist.
    pub l: Regs<U<8>, N>,
}

#[lower]
impl<const N: usize> Unit for Sorted<N> {
    async fn run(&mut self, (go, x): (In<Bit>, In<U<8>>), first: Out<U<8>>) {
        loop {
            DefaultClock::rising().await;
            let v = x.get();
            // Where the byte goes: after every byte not below it.
            let mut p = U::<8>::from(0u8);
            for j in 0..N {
                let ahead = Bit::from(self.l[j].get() >= v);
                p = p + mux(ahead, U::<8>::from(1u8), U::<8>::from(0u8));
            }
            let at0 = mux(Bit::from(p == 0), v, self.l[0].get());
            with!(self <= { go.get() ? l[0]: at0 });
            for j in 1..N {
                // Before p a place keeps its byte; at p it takes the new
                // one; after p it takes the byte of the place before.
                let w = mux(
                    Bit::from(p > j),
                    self.l[j].get(),
                    mux(Bit::from(p == j), v, self.l[j - 1].get()),
                );
                with!(self <= { go.get() ? l[j]: w });
            }
            first.set(self.l[0].get());
        }
    }
}
// end{unit}

fn main() {
    let (go_out, go) = signal::<Bit, DefaultClock>();
    let (x_out, x) = signal::<U<8>, DefaultClock>();
    let (first_out, first) = signal::<U<8>, DefaultClock>();
    let mut unit = Sorted::<4>::default();
    if let Some(mut w) = Wave::from_env() {
        w.clock::<DefaultClock>();
        w.add("go", &go);
        w.add("x", &x);
        w.add("sorted", &unit);
        w.add("first", &first);
        w.start();
    }
    let places = unit.l.0.to_vec();
    let mut sim = Running::new(unit.run((go, x), first_out));
    // Four bytes out of order, then one larger than all of them, which
    // pushes the smallest off the end.
    for v in [0x20u8, 0x40, 0x10, 0x30, 0x50] {
        go_out.set(Bit::One);
        x_out.set(U::from(v));
        sim.cycle();
        let now: Vec<u128> = places.iter().map(|r| r.get().raw()).collect();
        println!("in {v:#04x}: {now:x?}");
    }
    go_out.set(Bit::Zero);
    sim.cycle();
    let last: Vec<u128> = places.iter().map(|r| r.get().raw()).collect();
    assert_eq!(last, vec![0x50, 0x40, 0x30, 0x20]);
    println!("first {:#04x}", first.get().raw());
    stop();
    let net = Sorted::<4>::lowered("sorted");
    txhdl::netlist::write_vhdl_from_env(&net);
    print!("\n{}", net.verilog());
}
