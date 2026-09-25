// SPDX-License-Identifier: Apache-2.0
//! Ranges in a lowered body: a pattern `lo..=hi` or `lo..hi`, and
//! `(lo..=hi).contains(&x)` (issue 496).
//!
//! A byte is classed the way a parser of text would: a digit, a
//! capital, a small letter, or something else. Before, each range was
//! written as its members, `0x30 | 0x31 | ..`, or as a pair of
//! comparisons, and Clippy's lints for both were allowed in four places
//! with the reason beside them. A range pattern now lowers to
//! `lo <= x && x <= hi`, `lo..hi` to `x < hi` above, and `contains` the
//! same.
//!
//! Every class is asserted against Rust's own `is_ascii_*`, and the
//! netlist is checked against the run under nvc and Verilator.
use txhdl::comp::trace::{stop, Wave};
use txhdl::comp::{signal, Clock, DefaultClock, In, Out, Reg, Running, Unit};
use txhdl::types::{Bit, U};
use txhdl::{lower, select, with, Trace};

// begin{unit}
/// A byte's class three ways: by `select!`, by `match` and by
/// `contains`.
#[derive(Trace, Default)]
pub struct Class {
    pub by_select: Reg<U<2>>,
    pub by_match: Reg<U<2>>,
    pub digit: Reg<Bit>,
}

#[lower]
impl Unit for Class {
    async fn run(
        &mut self,
        c: In<U<8>>,
        (s, m, d): (Out<U<2>>, Out<U<2>>, Out<Bit>),
    ) {
        loop {
            DefaultClock::rising().await;
            let x = c.get();
            // 1 a digit, 2 a capital, 3 a small letter, 0 anything else.
            let by_select = select!(x.raw() => {
                0x30..=0x39 => U::<2>::from(1u8),
                0x41..=0x5a => U::<2>::from(2u8),
                0x61..=0x7a => U::<2>::from(3u8),
                _ => U::<2>::from(0u8),
            });
            // The same with the upper end left out, `lo..hi`.
            let by_match = match x.raw() {
                0x30..0x3a => U::<2>::from(1u8),
                0x41..0x5b => U::<2>::from(2u8),
                0x61..0x7b => U::<2>::from(3u8),
                _ => U::<2>::from(0u8),
            };
            let digit = Bit::from((0x30..=0x39).contains(&x.raw()));
            with!(self <= {
                by_select: by_select,
                by_match: by_match,
                digit: digit,
            });
            s.set(self.by_select.get());
            m.set(self.by_match.get());
            d.set(self.digit.get());
        }
    }
}
// end{unit}

/// The class Rust gives a byte.
fn class(b: u8) -> u8 {
    if b.is_ascii_digit() {
        1
    } else if b.is_ascii_uppercase() {
        2
    } else if b.is_ascii_lowercase() {
        3
    } else {
        0
    }
}

fn main() {
    let (c_out, c) = signal::<U<8>, DefaultClock>();
    let (s_out, s) = signal::<U<2>, DefaultClock>();
    let (m_out, m) = signal::<U<2>, DefaultClock>();
    let (d_out, d) = signal::<Bit, DefaultClock>();
    let mut unit = Class::default();
    if let Some(mut w) = Wave::from_env() {
        w.clock::<DefaultClock>();
        w.add("c", &c);
        w.add("classify", &unit);
        w.add("s", &s);
        w.add("m", &m);
        w.add("d", &d);
        w.start();
    }
    let mut sim = Running::new(unit.run(c, (s_out, m_out, d_out)));
    // Each edge of each range and a byte either side of it.
    let bytes = b"/09:@AZ[`az{ 5Qq~";
    for &b in bytes.iter() {
        c_out.set(U::from(b));
        sim.cycle();
        sim.cycle();
        let got = (s.get().raw() as u8, m.get().raw() as u8, d.get().to_bool());
        println!("{:?} {b:#04x}: {} {} {}", b as char, got.0, got.1, got.2);
        assert_eq!(got, (class(b), class(b), b.is_ascii_digit()), "{b:#04x}");
    }
    stop();
    let net = Class::lowered("classify");
    txhdl::netlist::write_vhdl_from_env(&net);
    print!("\n{}", net.verilog());
}
