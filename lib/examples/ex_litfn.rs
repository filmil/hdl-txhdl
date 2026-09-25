// SPDX-License-Identifier: Apache-2.0
//! A function under `#[lower]` called on a literal (issue 555).
//!
//! An SD card checks every command with a CRC-7, shifted in a bit at a
//! time. The first bit of a command goes into an empty register, so the
//! natural way to write it is the step applied to a zero:
//! `crc7_step(U::<7>::from(0u8), bit)`. The step reads the register's
//! top bit, and inlined on a literal that read was a bit of the
//! literal, which VHDL does not allow on a qualified expression:
//! `unsigned'("0000000")(6)` analysed in Verilator and was refused by
//! nvc. The lowering now folds a bit or a part of a literal into the
//! literal bit or bits it names, so neither netlist ever indexes one.
//! `top2` takes a part of a literal the same way.
//!
//! The run shifts in SD's CMD0, `40 00 00 00 00`, whose CRC-7 is
//! `0x4A`, and asserts it; the netlist is checked against the run under
//! nvc and Verilator.
use txhdl::comp::trace::{stop, Wave};
use txhdl::comp::{
    mux, signal, Clock, DefaultClock, In, Out, Reg, Running, Unit,
};
use txhdl::types::{Bit, U};
use txhdl::{lower, with, Trace};

// begin{fns}
/// One step of the CRC-7 SD uses, `x^7 + x^3 + 1`: the register shifted
/// up, and the polynomial folded in when the bit leaving the top
/// differs from the bit coming in.
#[lower]
fn crc7_step(crc: U<7>, bit: Bit) -> U<7> {
    let inv = bit ^ crc.bit(6);
    (crc << 1u32) ^ mux(inv, U::<7>::from(0x09u8), U::<7>::from(0u8))
}

/// The top two bits of a value.
#[lower]
fn top2(v: U<7>) -> U<2> {
    v.slice::<5, 2>()
}
// end{fns}

// begin{unit}
/// A CRC-7 over a serial stream: `start` marks a command's first bit.
#[derive(Trace, Default)]
pub struct Crc7 {
    pub crc: Reg<U<7>>,
}

#[lower]
impl Unit for Crc7 {
    async fn run(
        &mut self,
        (start, din): (In<Bit>, In<Bit>),
        (sum, tag): (Out<U<7>>, Out<U<2>>),
    ) {
        loop {
            DefaultClock::rising().await;
            let crc = self.crc.get();
            let b = din.get();
            // The first bit goes into an empty register: the step on
            // a literal, which is what issue 555 lowered wrongly.
            let first = crc7_step(U::<7>::from(0u8), b);
            let next = crc7_step(crc, b);
            with!(self <= { crc: mux(start.get(), first, next) });
            sum.set(crc);
            // A part of a literal: 0x60 is 1100000, whose top two
            // bits are 11.
            tag.set(top2(U::<7>::from(0x60u8)));
        }
    }
}
// end{unit}

/// SD's CMD0: a start bit, a transmission bit, index 0 and a zero
/// argument, forty bits.
const CMD0: [u8; 5] = [0x40, 0, 0, 0, 0];

fn main() {
    let (start_out, start) = signal::<Bit, DefaultClock>();
    let (din_out, din) = signal::<Bit, DefaultClock>();
    let (crc_out, crc) = signal::<U<7>, DefaultClock>();
    let (tag_out, tag) = signal::<U<2>, DefaultClock>();
    let mut unit = Crc7::default();
    if let Some(mut w) = Wave::from_env() {
        w.clock::<DefaultClock>();
        w.add("start", &start);
        w.add("din", &din);
        w.add("crc7", &unit);
        w.add("sum", &crc);
        w.add("tag", &tag);
        w.start();
    }
    let mut sim = Running::new(unit.run((start, din), (crc_out, tag_out)));
    // Twice, so the second command's first bit goes through the step on
    // the literal while the register still holds what the first left.
    for round in 0..2 {
        for i in 0..40 {
            let byte = CMD0[i / 8];
            let b = (byte >> (7 - i % 8)) & 1;
            start_out.set(if i == 0 { Bit::One } else { Bit::Zero });
            din_out.set(if b == 1 { Bit::One } else { Bit::Zero });
            sim.cycle();
        }
        start_out.set(Bit::Zero);
        din_out.set(Bit::Zero);
        sim.cycle();
        let got = crc.get().raw() as u8;
        println!("round {round}: CMD0's CRC-7 is {got:#04x}");
        assert_eq!(got, 0x4a, "CMD0's CRC-7 is 0x4A");
        assert_eq!(tag.get().raw(), 0b11, "the top two bits of 0x60");
    }
    stop();
    let net = Crc7::lowered("crc7");
    txhdl::netlist::write_vhdl_from_env(&net);
    print!("\n{}", net.verilog());
}
