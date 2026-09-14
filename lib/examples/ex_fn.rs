// SPDX-License-Identifier: Apache-2.0
//! A function under `#[lower]`: combinational, a few `let`s and a
//! value, plain Rust for the simulation and inlined at every call in
//! a lowered unit of the same file. The counter here keeps its two
//! pieces of arithmetic as functions of their own, the Gray code of
//! the count and whether the count is about to wrap, so the step
//! reads as what it does. Checked under nvc and Verilator against its
//! trace.
use txhdl::comp::trace::{stop, Wave};
use txhdl::comp::{
    now, signal, Clock, DefaultClock, In, Out, Reg, Running, Unit,
};
use txhdl::types::{Bit, U};
use txhdl::{lower, when, Trace};

// begin{fns}
/// The Gray code of a count: the count and itself shifted right,
/// exclusive or.
#[lower]
fn gray(n: U<4>) -> U<4> {
    n ^ (n >> 1)
}

/// Whether the count is the last before it wraps.
#[lower]
fn last(n: U<4>) -> Bit {
    Bit::from(n == 15)
}
// end{fns}

#[derive(Trace, Default)]
pub struct Gray {
    pub n: Reg<U<4>>,
}

#[lower]
impl Unit<In<Bit>, (Out<U<4>>, Out<Bit>)> for Gray {
    async fn run(
        &mut self,
        step: In<Bit>,
        (code, wrap): (Out<U<4>>, Out<Bit>),
    ) {
        loop {
            DefaultClock::rising().await;
            let n = self.n.get();
            let go = step.get();
            when!(go => { self.n <= n + 1 });
            code.set(gray(n));
            wrap.set(go & last(n));
        }
    }
}

fn main() {
    let (step_out, step) = signal::<Bit, DefaultClock>();
    let (code_out, code) = signal::<U<4>, DefaultClock>();
    let (wrap_out, wrap) = signal::<Bit, DefaultClock>();
    let mut gray = Gray::default();
    let n = gray.n.clone();
    if let Some(mut w) = Wave::from_env() {
        w.clock::<DefaultClock>();
        w.add("step", &step);
        w.add("gray", &gray);
        w.add("code", &code);
        w.add("wrap", &wrap);
        w.start();
    }
    let mut sim = Running::new(gray.run(step, (code_out, wrap_out)));
    // Steps on all but every fifth cycle: sixteen in twenty, one wrap.
    for c in 0..20 {
        step_out.set(c % 5 != 4);
        sim.cycle();
        println!(
            "t={:>2} n {:>2} code {:04b} wrap {}",
            now(),
            n.get().raw(),
            code.get().raw(),
            wrap.get().to_bool() as u8
        );
    }
    stop();
    txhdl::netlist::write_vhdl_from_env(&Gray::lowered("gray"));
    print!("\n{}", Gray::verilog("gray"));
}
