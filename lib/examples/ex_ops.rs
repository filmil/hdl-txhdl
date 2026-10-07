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
use txhdl::{lower, with, Trace};

/// Two bytes as signed numbers multiplied at 16 bits, and the product
/// sign-extended to 32: the shape the Tutorials session found lowered
/// to a bit of an expression, which Verilator refuses (issue 1369).
#[lower]
fn wide_product(a: U<8>, b: U<8>) -> U<32> {
    let p = a.sext::<16>().mul::<16>(b.sext::<16>());
    p.sext::<32>()
}

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
    pub tops: Reg<U<4>>,
    pub spread: Reg<U<8>>,
    pub wide: Reg<U<32>>,
    pub mid: Reg<U<4>>,
    pub nib: Reg<U<4>>,
    pub carry: Reg<Bit>,
    pub packed: Reg<U<9>>,
    pub spack: Reg<U<9>>,
    pub same_low: Reg<Bit>,
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
            with!(self <= {
                en ? {
                    sum: a + b,
                    diff: a - b,
                    both: a & b,
                    either: a | b,
                    differ: a ^ b,
                    up: a << 2,
                    down: (!a >> 1) & 0x3F,
                },
            });
            // A compare is a truth value. `&` joins it with a wire,
            // and a compare inside `&` is parenthesised, since `&`
            // binds tighter than `==` in Rust.
            let eq = a == b;
            let lt = a < b;
            let high = a >= 128;
            with!(self <= { en & (eq | lt) & !high ? hits: self.hits + 1 });
            // A slice of one bit is a value of one bit, and comparing
            // it with a number is how a program asks whether that bit
            // is set: through a name of its own for `a`, and inside
            // the compare for `b`.
            let top = a.slice::<7, 1>();
            with!(self <= {
                en & ((top == 1) | (b.slice::<7, 1>() == 1))
                    ? tops: self.tops + 1
            });
            // A computed value of one bit is a wire of one bit, and
            // sign-extending it spreads that bit over the width: zero,
            // or every bit set. In Verilog such a wire is a scalar,
            // which has no bit to select, so the extension has to
            // repeat the wire itself. See issue 247.
            let odd = a.slice::<0, 1>() ^ b.slice::<0, 1>();
            with!(self <= { en ? spread: odd.sext::<8>() });
            // A computed value whose bits are taken: a product sign
            // extended, which takes its top bit, inside a function the
            // lowering inlines, and a slice of a product and of a sum
            // and a bit of a sum, written in place. Verilog selects
            // bits of a name and not of an expression, so each value is
            // a wire of its own first (issue 1369).
            with!(self <= {
                en ? {
                    wide: wide_product(a, b),
                    mid: a.mul::<8>(b).slice::<2, 4>(),
                    nib: (a + b).slice::<4, 4>(),
                    carry: (a + b).bit(7),
                },
            });
            // A value narrowed inside a wider expression: a resize and
            // a sign extension to fewer bits keep the low bits, in a
            // concatenation, where a value not narrowed would push the
            // bit above it out, and in a comparison, where it would be
            // compared whole (issue 1387).
            let w = a.concat::<8, 16>(b);
            with!(self <= {
                en ? {
                    packed: U::<1>::from(1u8).concat::<8, 9>(w.resize::<8>()),
                    spack: U::<1>::from(1u8).concat::<8, 9>(w.sext::<8>()),
                    same_low: w.resize::<8>() == b,
                },
            });
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
    let (sum, hits, tops, spread) = (ops.sum, ops.hits, ops.tops, ops.spread);
    let (wide, mid, nib) = (ops.wide, ops.mid, ops.nib);
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
        // The last row is the only one whose top bit is on the second
        // input, so it is the one that counts through the compare
        // written inside the condition rather than through a name.
        (1, 200, true),
    ];
    for (x, y, e) in table {
        a_out.set(U::from(x));
        b_out.set(U::from(y));
        en_out.set(e);
        sim.cycle();
        println!(
            "a={x:3} b={y:3} en={} sum={:3} hits={} tops={} spread={:08b} \
             wide={:08x} mid={:x} nib={:x}",
            e as u8,
            sum.get().raw(),
            hits.get().raw(),
            tops.get().raw(),
            spread.get().raw(),
            wide.get().raw(),
            mid.get().raw(),
            nib.get().raw()
        );
    }
    sim.cycle();
    stop();
    txhdl::netlist::write_vhdl_from_env(&Ops::lowered("ops"));
    print!("\n{}", Ops::verilog("ops"));
}
