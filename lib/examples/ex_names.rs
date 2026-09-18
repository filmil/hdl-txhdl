// SPDX-License-Identifier: Apache-2.0
//! Every way a `let` can want a name the netlist has already given to
//! something else. The netlist has one namespace for fields, ports and
//! wires, and Rust's rules do not reach into it, so the lowering owns
//! that namespace: a computed `let` becomes a wire named as written
//! where the name is free, and `_w`, `_w2` where it is not (issue
//! 171). The netlist says which `let` became which wire, in a comment
//! above the module.
//!
//! The four cases are all here, and the run is checked against the
//! netlist under nvc and Verilator, so each of them is checked where
//! it is written:
//!
//! * `count` is a field of the unit and a computed `let` as well;
//! * `sum` is an output port and a computed `let`;
//! * `step` is bound twice, which Rust calls shadowing;
//! * `buffer`, which VHDL reserves, is a `let` as well.
//!
//! A `let` that only reads a register, a port or a number is an alias
//! and makes no wire, whatever it is called: `same` here reads the
//! register the field holds and takes no name in the netlist at all.
use txhdl::comp::trace::{stop, Wave};
use txhdl::comp::{
    now, signal, Clock, DefaultClock, In, Out, Reg, Running, Unit,
};
use txhdl::types::{Bit, U};
use txhdl::{lower, when, Trace};

// begin{unit}
/// A counter that adds a step and reports what it added.
#[derive(Trace, Default)]
pub struct Counter {
    pub count: Reg<U<8>>,
    pub last: Reg<U<8>>,
}

#[lower]
impl Unit for Counter {
    async fn run(
        &mut self,
        (go, step): (In<Bit>, In<U<4>>),
        (sum, odd): (Out<U<8>>, Out<Bit>),
    ) {
        loop {
            DefaultClock::rising().await;
            // An alias: a plain read is no wire, so the field's name is
            // free for it.
            let same = self.count.get();
            // A field has this name, so the wire is `count_w`.
            let count = same + step.get().zext::<8>();
            let total = count + self.last.get();
            sum.set(total);
            // A port has this name, so the wire is `sum_w`. In Rust the
            // `let` shadows the port as well, so the port is driven
            // above this line and read through the wire below it.
            let sum = total & 0x3f;
            // Bound twice, which Rust allows: the wires are `step_w`,
            // since the port took `step`, and `step_w2`; a read of
            // `step` after the second `let` sees the second wire.
            let step = step.get().zext::<8>() & 3;
            let step = step + 1;
            // A reserved word of VHDL: the wire is `buffer_w`.
            let buffer = sum ^ step;
            when!(go.get() => self { count: count, last: buffer });
            odd.set(Bit::from(buffer.slice::<0, 1>() == 1));
        }
    }
}
// end{unit}

fn main() {
    let (go_out, go) = signal::<Bit, DefaultClock>();
    let (step_out, step) = signal::<U<4>, DefaultClock>();
    let (sum_out, sum) = signal::<U<8>, DefaultClock>();
    let (odd_out, odd) = signal::<Bit, DefaultClock>();
    let mut counter = Counter::default();
    let count = counter.count;
    let last = counter.last;
    if let Some(mut w) = Wave::from_env() {
        w.clock::<DefaultClock>();
        w.add("go", &go);
        w.add("step", &step);
        w.add("counter", &counter);
        w.add("sum", &sum);
        w.add("odd", &odd);
        w.start();
    }
    let mut sim = Running::new(counter.run((go, step), (sum_out, odd_out)));
    for c in 0..10u32 {
        go_out.set(c % 4 != 3);
        step_out.set(U::<4>::from((c % 5) as u8));
        sim.cycle();
        println!(
            "t={:>2} count {:>3} last {:>3} sum {:>3} odd {}",
            now(),
            count.get().raw(),
            last.get().raw(),
            sum.get().raw(),
            odd.get().to_bool() as u8
        );
    }
    stop();
    txhdl::netlist::write_vhdl_from_env(&Counter::lowered("names"));
    print!("\n{}", Counter::verilog("names"));
}
