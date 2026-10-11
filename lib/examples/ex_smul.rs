// SPDX-License-Identifier: Apache-2.0
//! Products of extended operands at the operands' own widths (issue
//! 1649).
//!
//! A product of two 32-bit values sign-extended to 64 bits is a 32 by
//! 32 signed product, and the netlist says so: `$signed(a) * $signed(b)`
//! in Verilog and `signed(a) * signed(b)` in VHDL, rather than a 64 by
//! 64 product of the extended words, in which Vivado did not see the
//! operands' widths and built four DSP blocks in series. A zero-extended
//! operand beside a signed one is signed with a zero above it, so it
//! stays a positive number. A product narrower than its operands'
//! signed widths together, a byte by an unsigned byte into sixteen
//! bits, is written as before, since its signed product would have to
//! be cut, and VHDL's resize of a signed value does not cut it.
//!
//! Every product is asserted against the same arithmetic on Rust's
//! integers, and the netlist is checked against the run under nvc and
//! Verilator.
use txhdl::comp::trace::{stop, Wave};
use txhdl::comp::{signal, Clock, DefaultClock, In, Out, Reg, Running, Unit};
use txhdl::types::U;
use txhdl::{lower, with, Trace};

// begin{unit}
/// Three products, registered: two signed words, a signed word and an
/// unsigned half, and a signed byte and an unsigned byte into sixteen
/// bits, which stays as it was.
#[derive(Trace, Default)]
pub struct Smul {
    pub pss: Reg<U<64>>,
    pub psu: Reg<U<64>>,
    pub psmall: Reg<U<16>>,
}

#[lower]
impl Unit for Smul {
    async fn run(
        &mut self,
        (a, b, h, c, d): (In<U<32>>, In<U<32>>, In<U<16>>, In<U<8>>, In<U<8>>),
        (ss, su, small): (Out<U<64>>, Out<U<64>>, Out<U<16>>),
    ) {
        loop {
            DefaultClock::rising().await;
            let x = a.get().sext::<64>();
            with!(self <= {
                pss: x.mul::<64>(b.get().sext::<64>()),
                psu: x.mul::<64>(h.get().resize::<64>()),
                psmall: c.get().sext::<16>().mul::<16>(d.get().resize::<16>()),
            });
            ss.set(self.pss.get());
            su.set(self.psu.get());
            small.set(self.psmall.get());
        }
    }
}
// end{unit}

/// Operands: signs both ways, the most negative values, and the largest
/// unsigned ones, which a product read as signed would get wrong.
const CASES: [(i32, i32, u16, i8, u8); 6] = [
    (3, 4, 5, 3, 7),
    (-3, 4, 5, -3, 7),
    (-7, -9, 0xffff, -128, 255),
    (i32::MIN, i32::MIN, 0xffff, -128, 128),
    (i32::MAX, -1, 0x8000, 127, 255),
    (0x1234_5678, -0x0765_4321, 0x7fff, -1, 1),
];

fn main() {
    let (a_out, a) = signal::<U<32>, DefaultClock>();
    let (b_out, b) = signal::<U<32>, DefaultClock>();
    let (h_out, h) = signal::<U<16>, DefaultClock>();
    let (c_out, c) = signal::<U<8>, DefaultClock>();
    let (d_out, d) = signal::<U<8>, DefaultClock>();
    let (ss_out, ss) = signal::<U<64>, DefaultClock>();
    let (su_out, su) = signal::<U<64>, DefaultClock>();
    let (small_out, small) = signal::<U<16>, DefaultClock>();
    let mut unit = Smul::default();
    if let Some(mut w) = Wave::from_env() {
        w.clock::<DefaultClock>();
        w.add("a", &a);
        w.add("b", &b);
        w.add("h", &h);
        w.add("c", &c);
        w.add("d", &d);
        w.add("smul", &unit);
        w.add("ss", &ss);
        w.add("su", &su);
        w.add("small", &small);
        w.start();
    }
    let mut sim =
        Running::new(unit.run((a, b, h, c, d), (ss_out, su_out, small_out)));
    // The operands go in, are multiplied at the next edge, and are on
    // the outputs the edge after.
    for &(x, y, z, p, q) in CASES.iter() {
        a_out.set(U::from(x as u32));
        b_out.set(U::from(y as u32));
        h_out.set(U::from(z));
        c_out.set(U::from(p as u8));
        d_out.set(U::from(q));
        sim.cycle();
        sim.cycle();
        let got = (
            ss.get().raw() as u64,
            su.get().raw() as u64,
            small.get().raw() as u16,
        );
        let want = (
            (i64::from(x) * i64::from(y)) as u64,
            (i64::from(x) * i64::from(z)) as u64,
            (i16::from(p) * i16::from(q)) as u16,
        );
        println!(
            "{x:11} * {y:11} = {:21}, * {z:5} = {:21}; \
             {p:4} * {q:3} = {:6}",
            got.0 as i64, got.1 as i64, got.2 as i16
        );
        assert_eq!(got, want, "{x}, {y}, {z}, {p}, {q}");
    }
    stop();
    let net = Smul::lowered("smul");
    txhdl::netlist::write_vhdl_from_env(&net);
    print!("\n{}", net.verilog());
}
