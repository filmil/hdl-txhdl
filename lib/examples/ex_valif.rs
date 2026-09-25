// SPDX-License-Identifier: Apache-2.0
//! An `if` and a `match` that yield a value, in a lowered body (issue
//! 496).
//!
//! Rust chooses a value with `if` and `match`, and until now a lowered
//! body had to say it with `mux` and `select!`. Both now lower to the
//! same chain of conditions: an `if` with an `else`, an `else if`
//! after it, and a `match` whose arms are numbers, alternatives with
//! `|` and a default, one arm a block. A branch is one expression, and
//! an `if` with no `else` has no value when its condition is false, so
//! each is refused with a message saying what to write.
//!
//! Every result is asserted against the same choice made in Rust, and
//! the netlist is checked against the run under nvc and Verilator.
use txhdl::comp::trace::{stop, Wave};
use txhdl::comp::{signal, Clock, DefaultClock, In, Out, Reg, Running, Unit};
use txhdl::types::U;
use txhdl::{lower, with, Trace};

// begin{unit}
/// Two choices of one of two operands, or a mix of them, by `op`.
#[derive(Trace, Default)]
pub struct Choose {
    pub by_if: Reg<U<8>>,
    pub by_match: Reg<U<8>>,
}

#[lower]
impl Unit for Choose {
    async fn run(
        &mut self,
        (op, a, b): (In<U<2>>, In<U<8>>, In<U<8>>),
        (i, m): (Out<U<8>>, Out<U<8>>),
    ) {
        loop {
            DefaultClock::rising().await;
            let o = op.get();
            let x = a.get();
            let y = b.get();
            let by_if = if o == 0 {
                x
            } else if o == 1 {
                y
            } else {
                x ^ y
            };
            let by_match = match o.raw() {
                0 => y,
                1 | 2 => x + y,
                _ => x & y,
            };
            with!(self <= { by_if: by_if, by_match: by_match });
            i.set(self.by_if.get());
            m.set(self.by_match.get());
        }
    }
}
// end{unit}

/// What the unit should choose, in plain Rust.
fn want(op: u8, x: u8, y: u8) -> (u8, u8) {
    let by_if = match op {
        0 => x,
        1 => y,
        _ => x ^ y,
    };
    let by_match = match op {
        0 => y,
        1 | 2 => x.wrapping_add(y),
        _ => x & y,
    };
    (by_if, by_match)
}

fn main() {
    let (op_out, op) = signal::<U<2>, DefaultClock>();
    let (a_out, a) = signal::<U<8>, DefaultClock>();
    let (b_out, b) = signal::<U<8>, DefaultClock>();
    let (i_out, i) = signal::<U<8>, DefaultClock>();
    let (m_out, m) = signal::<U<8>, DefaultClock>();
    let mut unit = Choose::default();
    if let Some(mut w) = Wave::from_env() {
        w.clock::<DefaultClock>();
        w.add("op", &op);
        w.add("a", &a);
        w.add("b", &b);
        w.add("choose", &unit);
        w.add("i", &i);
        w.add("m", &m);
        w.start();
    }
    let mut sim = Running::new(unit.run((op, a, b), (i_out, m_out)));
    for (k, &(x, y)) in
        [(0x0f, 0xf0), (200, 100), (0xaa, 0x0f)].iter().enumerate()
    {
        for o in 0..4u8 {
            op_out.set(U::from(o));
            a_out.set(U::from(x));
            b_out.set(U::from(y));
            sim.cycle();
            sim.cycle();
            let got = (i.get().raw() as u8, m.get().raw() as u8);
            println!(
                "{k}: op {o} a {x:3} b {y:3}: if {:3} match {:3}",
                got.0, got.1
            );
            assert_eq!(got, want(o, x, y), "op {o}, {x} and {y}");
        }
    }
    stop();
    let net = Choose::lowered("choose");
    txhdl::netlist::write_vhdl_from_env(&net);
    print!("\n{}", net.verilog());
}
