// SPDX-License-Identifier: Apache-2.0
//! What a unit states about itself: a check, an assumption and a cover
//! point, checked by the run and written into the netlist.
//!
//! Every other example is checked by comparison: the netlist must do
//! what the run did. That says nothing about a design whose Rust is
//! wrong in the same way as its netlist. `check!`, `assume!` and
//! `cover!` state what must hold, what the unit takes for granted, and
//! what can happen, where the design is written (issue 502).
//!
//! The counter below counts in decimal: it adds a step of nought, one
//! or two and wraps past nine. It checks that its count is always a
//! digit, and that a wrap lands below two; it assumes its step is never
//! three, which it does not handle; and it covers the count reaching
//! nine. The run checks each as it goes and counts the cover point.
//!
//! The netlist has them too. In the VHDL a check and an assumption are
//! VHDL's own `assert`, so nvc checks them while it replays the run; in
//! the Verilog they are SystemVerilog's immediate assertions, between
//! `ifdef FORMAL` and `endif`, which Verilator checks here and which a
//! formal tool can prove for every input.
use txhdl::comp::trace::{stop, Wave};
use txhdl::comp::{
    mux, now, signal, Clock, DefaultClock, In, Out, Reg, Running, Unit,
};
use txhdl::formal::covered;
use txhdl::types::{Bit, U};
use txhdl::{assume, check, cover, lower, with, Trace};

// begin{unit}
/// A decimal digit that counts by the step it is given.
#[derive(Trace, Default)]
pub struct Digit {
    pub count: Reg<U<4>>,
}

#[lower]
impl Unit for Digit {
    async fn run(
        &mut self,
        step: In<U<2>>,
        (value, wrap): (Out<U<4>>, Out<Bit>),
    ) {
        loop {
            DefaultClock::rising().await;
            let count = self.count.get();
            let step = step.get();
            assume!(step != 3, "the step is nought, one or two");
            check!(count <= 9, "the count is a digit");
            cover!(count == 9, "the count reaches nine");
            let sum = count + step.zext::<4>();
            let over = Bit::from(sum > 9);
            let next = mux(over, sum - 10, sum);
            if over.to_bool() {
                check!(next <= 1, "a wrap lands on nought or one");
            }
            with!(self <= {
                Bit::One ? count: next,
            });
            value.set(count);
            wrap.set(over);
        }
    }
}
// end{unit}

fn main() {
    let (step_out, step) = signal::<U<2>, DefaultClock>();
    let (value_out, value) = signal::<U<4>, DefaultClock>();
    let (wrap_out, wrap) = signal::<Bit, DefaultClock>();
    let mut digit = Digit::default();
    let count = digit.count;
    if let Some(mut w) = Wave::from_env() {
        w.clock::<DefaultClock>();
        w.add("step", &step);
        w.add("digit", &digit);
        w.add("value", &value);
        w.add("wrap", &wrap);
        w.start();
    }
    let mut sim = Running::new(digit.run(step, (value_out, wrap_out)));
    for c in 0..16u32 {
        step_out.set(U::<2>::from((c % 3) as u8));
        sim.cycle();
        println!(
            "t={:>2} step {} count {} wrap {}",
            now(),
            c % 3,
            count.get().raw(),
            wrap.get().to_bool() as u8
        );
    }
    stop();
    let nines = covered("the count reaches nine");
    println!("the count reached nine {nines} times");
    assert!(nines > 0, "the cover point was reached");
    let net = Digit::lowered("digit");
    txhdl::netlist::write_vhdl_from_env(&net);
    print!("\n{}", net.verilog());
}
