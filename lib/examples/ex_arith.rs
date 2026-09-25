// SPDX-License-Identifier: Apache-2.0
//! Multiplication, remainder and negation in a lowered body (issue
//! 496).
//!
//! `*`, `%` and unary `-` are Rust's operators on a value, at its
//! width and wrapping as `+` and `-` are, and they lower to the
//! netlist's: `*` cut to the width, `%` as `rem` in VHDL, and `-x` as
//! `~x + 1`, the two's complement Rust wraps to. Before, a product was
//! `.mul::<M>()` and a negation a subtraction from zero. A `-` after
//! another operator is unary, so `a + -b` is a sum.
//!
//! Every product, remainder and negation is asserted against the same
//! arithmetic on `u8`, and the netlist is checked against the run under
//! nvc and Verilator.
use txhdl::comp::trace::{stop, Wave};
use txhdl::comp::{signal, Clock, DefaultClock, In, Out, Reg, Running, Unit};
use txhdl::types::U;
use txhdl::{lower, with, Trace};

// begin{unit}
/// One of each, registered, from two operands.
#[derive(Trace, Default)]
pub struct Arith {
    pub prod: Reg<U<8>>,
    pub rest: Reg<U<8>>,
    pub neg: Reg<U<8>>,
    pub sum: Reg<U<8>>,
}

#[lower]
impl Unit for Arith {
    async fn run(
        &mut self,
        (a, b): (In<U<8>>, In<U<8>>),
        (p, r, n, s): (Out<U<8>>, Out<U<8>>, Out<U<8>>, Out<U<8>>),
    ) {
        loop {
            DefaultClock::rising().await;
            let x = a.get();
            let y = b.get();
            with!(self <= {
                prod: x * y,
                rest: x % y,
                neg: -x,
                sum: x + -y,
            });
            p.set(self.prod.get());
            r.set(self.rest.get());
            n.set(self.neg.get());
            s.set(self.sum.get());
        }
    }
}
// end{unit}

/// Operand pairs, the right one never zero, since a remainder by zero
/// is refused.
const PAIRS: [(u8, u8); 8] = [
    (7, 3),
    (200, 3),
    (16, 16),
    (255, 255),
    (13, 1),
    (0, 9),
    (128, 2),
    (99, 250),
];

fn main() {
    let (a_out, a) = signal::<U<8>, DefaultClock>();
    let (b_out, b) = signal::<U<8>, DefaultClock>();
    let (p_out, p) = signal::<U<8>, DefaultClock>();
    let (r_out, r) = signal::<U<8>, DefaultClock>();
    let (n_out, n) = signal::<U<8>, DefaultClock>();
    let (s_out, s) = signal::<U<8>, DefaultClock>();
    let mut unit = Arith::default();
    if let Some(mut w) = Wave::from_env() {
        w.clock::<DefaultClock>();
        w.add("a", &a);
        w.add("b", &b);
        w.add("arith", &unit);
        w.add("p", &p);
        w.add("r", &r);
        w.add("n", &n);
        w.add("s", &s);
        w.start();
    }
    let mut sim = Running::new(unit.run((a, b), (p_out, r_out, n_out, s_out)));
    // An operand pair goes in, is registered at the next edge, and is
    // on the outputs the edge after.
    for &(x, y) in PAIRS.iter() {
        a_out.set(U::from(x));
        b_out.set(U::from(y));
        sim.cycle();
        sim.cycle();
        let got = (
            p.get().raw() as u8,
            r.get().raw() as u8,
            n.get().raw() as u8,
            s.get().raw() as u8,
        );
        let want = (
            x.wrapping_mul(y),
            x % y,
            0u8.wrapping_sub(x),
            x.wrapping_add(0u8.wrapping_sub(y)),
        );
        println!(
            "{x:3} {y:3}: * {:3} % {:3} -a {:3} a+-b {:3}",
            got.0, got.1, got.2, got.3
        );
        assert_eq!(got, want, "{x} and {y}");
    }
    stop();
    let net = Arith::lowered("arith");
    txhdl::netlist::write_vhdl_from_env(&net);
    print!("\n{}", net.verilog());
}
