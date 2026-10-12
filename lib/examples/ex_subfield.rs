// SPDX-License-Identifier: Apache-2.0
//! Fields of nested structs read in a lowered unit (issue 1648).
//!
//! A beat holds two pairs of halves and a flag, `Beat { h, g, last }`,
//! each pair one `Halves { hi, lo }`, so the struct of halves is written
//! once and used twice. The unit reads `p.get().h.lo` and
//! `p.get().g.hi`, a field of a field of the port's value; the lowering
//! finds the bits through each field's type, `lo` inside `Halves` at
//! `h`'s place inside `Beat`, as it finds one field through one type.
//!
//! The run asserts each field against the beat it came from, and the
//! netlist is checked against the run under nvc and Verilator.
use txhdl::comp::trace::{stop, Wave};
use txhdl::comp::{signal, Clock, DefaultClock, In, Out, Reg, Running, Unit};
use txhdl::types::{Bit, U};
use txhdl::{lower, with, Trace, Value};

// begin{unit}
/// Two halves of a sixteen-bit value.
#[derive(Value, Clone, Copy, Default, Debug, PartialEq)]
pub struct Halves {
    pub hi: U<8>,
    pub lo: U<8>,
}

/// A beat: two pairs of halves and whether it is the last.
#[derive(Value, Clone, Copy, Default, Debug, PartialEq)]
pub struct Beat {
    pub h: Halves,
    pub g: Halves,
    pub last: Bit,
}

/// The low half of `h`, the high half of `g`, and the flag, registered.
#[derive(Trace, Default)]
pub struct Subfield {
    pub rlo: Reg<U<8>>,
    pub rhi: Reg<U<8>>,
    pub rlast: Reg<Bit>,
}

#[lower]
impl Unit for Subfield {
    async fn run(
        &mut self,
        p: In<Beat>,
        (lo, hi, last): (Out<U<8>>, Out<U<8>>, Out<Bit>),
    ) {
        loop {
            DefaultClock::rising().await;
            with!(self <= {
                rlo: p.get().h.lo,
                rhi: p.get().g.hi,
                rlast: p.get().last,
            });
            lo.set(self.rlo.get());
            hi.set(self.rhi.get());
            last.set(self.rlast.get());
        }
    }
}
// end{unit}

/// Beats whose four bytes differ, so a field read from the wrong place
/// shows.
const BEATS: [(u8, u8, u8, u8, bool); 4] = [
    (0x12, 0x34, 0x56, 0x78, false),
    (0xa1, 0xb2, 0xc3, 0xd4, true),
    (0xff, 0x00, 0x01, 0xfe, false),
    (0x0f, 0xf0, 0x55, 0xaa, true),
];

fn main() {
    let (p_out, p) = signal::<Beat, DefaultClock>();
    let (lo_out, lo) = signal::<U<8>, DefaultClock>();
    let (hi_out, hi) = signal::<U<8>, DefaultClock>();
    let (last_out, last) = signal::<Bit, DefaultClock>();
    let mut unit = Subfield::default();
    if let Some(mut w) = Wave::from_env() {
        w.clock::<DefaultClock>();
        w.add("p", &p);
        w.add("subfield", &unit);
        w.add("lo", &lo);
        w.add("hi", &hi);
        w.add("last", &last);
        w.start();
    }
    let mut sim = Running::new(unit.run(p, (lo_out, hi_out, last_out)));
    // A beat goes in, its fields are registered at the next edge, and
    // are on the outputs the edge after.
    for &(hh, hl, gh, gl, l) in BEATS.iter() {
        p_out.set(Beat {
            h: Halves {
                hi: U::from(hh),
                lo: U::from(hl),
            },
            g: Halves {
                hi: U::from(gh),
                lo: U::from(gl),
            },
            last: if l { Bit::One } else { Bit::Zero },
        });
        sim.cycle();
        sim.cycle();
        let got = (lo.get().raw() as u8, hi.get().raw() as u8, last.get());
        println!(
            "h = {hh:02x}{hl:02x}, g = {gh:02x}{gl:02x}, last = {l:5}: \
             h.lo = {:02x}, g.hi = {:02x}, last = {:?}",
            got.0, got.1, got.2
        );
        assert_eq!(got.0, hl, "h.lo");
        assert_eq!(got.1, gh, "g.hi");
        assert_eq!(got.2 == Bit::One, l, "last");
    }
    stop();
    let net = Subfield::lowered("subfield");
    txhdl::netlist::write_vhdl_from_env(&net);
    print!("\n{}", net.verilog());
}
