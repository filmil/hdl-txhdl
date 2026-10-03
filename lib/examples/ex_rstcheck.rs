// SPDX-License-Identifier: Apache-2.0
//! A check of a unit that takes its own reset, stated only out of
//! reset (issue 633).
//!
//! The decade counter below reads `rst` itself: it clears its count on
//! it, and while it is held it shows a blank, `0xff`, on its output
//! instead of a digit. It checks that what it shows is a digit.
//!
//! That check means "once out of reset". Under reset the output is a
//! blank on purpose, and a check of it there would fail for no fault
//! of the design. So the netlist states it only while `rst` is low, in
//! the Verilog as `if (!rst) assert (..)` and in the VHDL as `assert
//! rst = '1' or (..)`, whichever side declared the port. Before issue
//! 633 it did that only for a reset the netlist added; a unit that
//! declared its own, as this one and the core do, had its checks
//! stated in reset too, and a proof or a simulation through a reset
//! failed on them.
//!
//! The run gates the check on the reset as the netlist does (issue
//! 879). It counts, holds the reset for three cycles, during which
//! the output is a blank and the check is not stated, and counts past
//! a wrap; the port and the reset the netlist gives every module are
//! one net, so it drives both. Two more testbenches in //docs, one
//! per language, hold the reset on their own and count on.
use txhdl::comp::trace::{stop, Wave};
use txhdl::comp::{
    mux, now, set_reset, signal, Clock, DefaultClock, In, Out, Reg, Running,
    Unit,
};
use txhdl::types::{Bit, U};
use txhdl::{check, lower, Trace};

// begin{unit}
/// A decade counter that clears on its own reset and shows a blank
/// while it is held.
#[derive(Trace, Default)]
pub struct Decade {
    pub count: Reg<U<8>>,
}

#[lower]
impl Unit<(In<Bit>, In<Bit>), Out<U<8>>> for Decade {
    async fn run(&mut self, (rst, tick): (In<Bit>, In<Bit>), shown: Out<U<8>>) {
        loop {
            DefaultClock::rising().await;
            let count = self.count.get();
            let blank = mux(rst.get(), U::<8>::new(0xff), count);
            check!(blank <= 9, "what is shown is a digit");
            let up = mux(Bit::from(count == 9), U::<8>::new(0), count + 1);
            let next = mux(tick.get(), up, count);
            self.count.set(mux(rst.get(), U::<8>::new(0), next));
            shown.set(blank);
        }
    }
}
// end{unit}

fn main() {
    let (rst_out, rst) = signal::<Bit, DefaultClock>();
    let (tick_out, tick) = signal::<Bit, DefaultClock>();
    let (shown_out, shown) = signal::<U<8>, DefaultClock>();
    let mut decade = Decade::default();
    if let Some(mut wave) = Wave::from_env() {
        wave.clock::<DefaultClock>();
        wave.add("rst", &rst);
        wave.add("tick", &tick);
        wave.add("shown", &shown);
        wave.add("decade", &decade);
        wave.start();
    }
    let mut sim = Running::new(decade.run((rst, tick), shown_out));

    // Counting, out of reset.
    rst_out.set(Bit::Zero);
    tick_out.set(Bit::One);
    for _ in 0..4 {
        sim.cycle();
    }
    println!("t={:>2} shown {}", now(), shown.get().raw());
    // The reset, three cycles: a blank shown, and no check stated.
    set_reset(true);
    rst_out.set(Bit::One);
    for _ in 0..3 {
        sim.cycle();
    }
    println!("t={:>2} shown {:#x} (in reset)", now(), shown.get().raw());
    // Out of it, and counting past a wrap.
    set_reset(false);
    rst_out.set(Bit::Zero);
    for _ in 0..14 {
        sim.cycle();
    }
    println!("t={:>2} shown {}", now(), shown.get().raw());

    print!("\n{}", Decade::verilog("decade"));
    stop();
    txhdl::netlist::write_vhdl_from_env(&Decade::lowered("decade"));
}
