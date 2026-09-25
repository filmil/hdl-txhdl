// SPDX-License-Identifier: Apache-2.0
//! Helpers from another file, taking and giving a struct.
//!
//! A function under `#[lower]` is inlined into the units that call it,
//! and it used to be found by reading the unit's own file, so a helper
//! had to live beside every unit that used it, and took each value as a
//! parameter of its own: a helper of fourteen registers had fourteen
//! parameters. Now a helper may be `pub` in any module of the crate or
//! of a dependency, and may take and give a struct (issue 504).
//!
//! The helpers here are in `helpers/codes.rs`, a module of this crate
//! the unit's file does not hold. `swap` takes a `Pair` and gives one
//! back; `mix` reads its `Pair` twice and calls `gray` twice; the unit
//! calls all three. The build checks the netlist against the run under
//! nvc and Verilator.
#[path = "helpers/codes.rs"]
mod codes;

use codes::{gray, mix, swap, Pair};
use txhdl::comp::trace::{stop, Wave};
use txhdl::comp::{
    now, signal, Clock, DefaultClock, In, Out, Reg, Running, Unit,
};
use txhdl::types::{Bit, U};
use txhdl::{lower, with, Trace};

// begin{unit}
/// A byte pair's codes: the first byte's Gray code, the pair swapped,
/// and the swapped pair mixed, held in a register.
#[derive(Trace, Default)]
pub struct Coder {
    pub held: Reg<U<8>>,
}

#[lower]
impl Unit for Coder {
    async fn run(
        &mut self,
        (a, b): (In<U<8>>, In<U<8>>),
        (code, swapped, mixed): (Out<U<8>>, Out<Pair>, Out<U<8>>),
    ) {
        loop {
            DefaultClock::rising().await;
            let p = Pair {
                hi: a.get(),
                lo: b.get(),
            };
            let q = swap(p);
            code.set(gray(a.get()));
            swapped.set(q);
            with!(self <= {
                Bit::One ? held: mix(q),
            });
            mixed.set(self.held.get());
        }
    }
}
// end{unit}

fn main() {
    let (a_out, a) = signal::<U<8>, DefaultClock>();
    let (b_out, b) = signal::<U<8>, DefaultClock>();
    let (code_out, code) = signal::<U<8>, DefaultClock>();
    let (swapped_out, swapped) = signal::<Pair, DefaultClock>();
    let (mixed_out, mixed) = signal::<U<8>, DefaultClock>();
    let mut coder = Coder::default();
    if let Some(mut w) = Wave::from_env() {
        w.clock::<DefaultClock>();
        w.add("a", &a);
        w.add("b", &b);
        w.add("coder", &coder);
        w.add("code", &code);
        w.add("swapped", &swapped);
        w.add("mixed", &mixed);
        w.start();
    }
    let mut sim =
        Running::new(coder.run((a, b), (code_out, swapped_out, mixed_out)));
    for c in 0..8u32 {
        a_out.set(U::<8>::from((c * 29 + 3) as u8));
        b_out.set(U::<8>::from((c * 71 + 11) as u8));
        sim.cycle();
        let s = swapped.get();
        println!(
            "t={:>2} code {:#04x} swapped {:#04x}{:02x} mixed {:#04x}",
            now(),
            code.get().raw(),
            s.hi.raw(),
            s.lo.raw(),
            mixed.get().raw()
        );
    }
    stop();
    let net = Coder::lowered("coder");
    txhdl::netlist::write_vhdl_from_env(&net);
    print!("\n{}", net.verilog());
}
