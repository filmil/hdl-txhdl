// SPDX-License-Identifier: Apache-2.0
//! A child's input held at a constant.
//!
//! A unit of units joins its children's ports to its own and to the
//! channels and wires it makes. Sometimes an input has nothing to be
//! joined to: the design wants it at a fixed value, a step of three or
//! an enable that is always on. `tie(v)` is that value where the input
//! is passed (issue 498). In Rust it is a wire set once and never
//! driven again; in the parent's netlist it is a wire driven by the
//! constant, joined to the child's input.
//!
//! Before this, a design made a `signal` it never drove and passed its
//! reading end, which holds the type's default: a constant, but always
//! zero, and nothing in the source said so.
//!
//! The netlist of the parent is checked against this run under nvc and
//! Verilator, so the two constants are checked where they are used.
use txhdl::comp::trace::{stop, Wave};
use txhdl::comp::{
    now, signal, tie, Clock, DefaultClock, In, Out, Reg, Running, Unit,
};
use txhdl::types::{Bit, U};
use txhdl::{lower, with, Trace};

// begin{unit}
/// Adds its input and a step to a total, while it is enabled.
#[derive(Trace, Default)]
pub struct Acc {
    pub total: Reg<U<8>>,
}

#[lower]
impl Unit for Acc {
    async fn run(
        &mut self,
        (x, step, en): (In<U<8>>, In<U<8>>, In<Bit>),
        y: Out<U<8>>,
    ) {
        loop {
            DefaultClock::rising().await;
            let total = self.total.get();
            with!(self <= {
                en.get() ? total: total + x.get() + step.get(),
            });
            y.set(total);
        }
    }
}

/// The accumulator with its step held at three and its enable on.
#[derive(Trace, Default)]
pub struct Stepped {
    pub acc: Acc,
}

#[lower]
impl Unit for Stepped {
    async fn run(&mut self, x: In<U<8>>, y: Out<U<8>>) {
        self.acc
            .run((x, tie(U::<8>::from(3u8)), tie(Bit::One)), y)
            .await;
    }
}
// end{unit}

fn main() {
    let (x_out, x) = signal::<U<8>, DefaultClock>();
    let (y_out, y) = signal::<U<8>, DefaultClock>();
    let mut top = Stepped::default();
    let total = top.acc.total;
    if let Some(mut w) = Wave::from_env() {
        w.clock::<DefaultClock>();
        w.add("x", &x);
        w.add("stepped", &top);
        w.add("y", &y);
        w.start();
    }
    let mut sim = Running::new(top.run(x, y_out));
    for c in 0..8u32 {
        x_out.set(U::<8>::from((c * 10) as u8));
        sim.cycle();
        println!(
            "t={:>2} total {:>3} y {:>3}",
            now(),
            total.get().raw(),
            y.get().raw()
        );
    }
    stop();
    let net = Stepped::lowered("stepped");
    txhdl::netlist::write_vhdl_from_env(&net);
    print!("\n{}", net.verilog());
}
