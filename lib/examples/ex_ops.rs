// SPDX-License-Identifier: Apache-2.0
//! The operators are Rust's. `+` and `-` wrap at the width; `&`, `|`,
//! `^` and `!` are bitwise; `<<` and `>>` shift by an integer; a
//! compare yields a `bool`, and a `bool` is a condition wherever a
//! `Bit` is, and joins one with `&` and `|`. A literal on the right
//! needs no wrapper. Each lowers to the operator of the same name,
//! and the netlist is simulated against this run's trace.
use txhdl::comp::trace::{stop, Wave};
use txhdl::comp::{signal, Clock, DefaultClock, In, Out, Reg, Running, Unit};
use txhdl::types::{Bit, U};
use txhdl::{lower, when, Trace};

#[derive(Trace, Default)]
pub struct Ops {
    pub sum: Reg<U<8>>,
    pub diff: Reg<U<8>>,
    pub both: Reg<U<8>>,
    pub either: Reg<U<8>>,
    pub differ: Reg<U<8>>,
    pub up: Reg<U<8>>,
    pub down: Reg<U<8>>,
    pub hits: Reg<U<4>>,
}

#[lower]
impl Unit for Ops {
    async fn run(
        &mut self,
        (a, b, en): (In<U<8>>, In<U<8>>, In<Bit>),
        (same, below): (Out<Bit>, Out<Bit>),
    ) {
        loop {
            DefaultClock::rising().await;
            let (a, b, en) = (a.get(), b.get(), en.get());
            // Arithmetic and logic on values; the shift amount and
            // the mask are literals.
            when!(en => {
                self.sum <= a + b;
                self.diff <= a - b;
                self.both <= a & b;
                self.either <= a | b;
                self.differ <= a ^ b;
                self.up <= a << 2;
                self.down <= (!a >> 1) & 0x3F
            });
            // A compare is a truth value. `&` joins it with a wire,
            // and a compare inside `&` is parenthesised, since `&`
            // binds tighter than `==` in Rust.
            let eq = a == b;
            let lt = a < b;
            let high = a >= 128;
            when!(en & (eq | lt) & !high => { self.hits <= self.hits + 1 });
            same.set(eq);
            below.set(lt & !eq);
        }
    }
}

fn main() {
    let (a_out, a) = signal::<U<8>, DefaultClock>();
    let (b_out, b) = signal::<U<8>, DefaultClock>();
    let (en_out, en) = signal::<Bit, DefaultClock>();
    let (same_out, same) = signal::<Bit, DefaultClock>();
    let (below_out, below) = signal::<Bit, DefaultClock>();
    let mut ops = Ops::default();
    let (sum, hits) = (ops.sum, ops.hits);
    if let Some(mut w) = Wave::from_env() {
        w.clock::<DefaultClock>();
        w.add("a", &a);
        w.add("b", &b);
        w.add("en", &en);
        w.add("ops", &ops);
        w.add("same", &same);
        w.add("below", &below);
        w.start();
    }
    let mut sim = Running::new(ops.run((a, b, en), (same_out, below_out)));
    let table: [(u8, u8, bool); 8] = [
        (7, 5, true),
        (5, 7, true),
        (9, 9, true),
        (250, 10, true),
        (200, 1, true),
        (3, 4, false),
        (0xF0, 0x3C, true),
        (1, 1, true),
    ];
    for (x, y, e) in table {
        a_out.set(U::from(x));
        b_out.set(U::from(y));
        en_out.set(e);
        sim.cycle();
        println!(
            "a={x:3} b={y:3} en={} sum={:3} hits={}",
            e as u8,
            sum.get().raw(),
            hits.get().raw()
        );
    }
    sim.cycle();
    stop();
    txhdl::netlist::write_vhdl_from_env(&Ops::lowered("ops"));
    print!("\n{}", Ops::verilog("ops"));
}
