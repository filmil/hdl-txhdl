// SPDX-License-Identifier: Apache-2.0
//! An array as a field of a value, read by index in a lowered body
//! (issue 1869).
//!
//! `Pixel` is four lanes of a byte and a flag, `lanes: [U<8>; 4]`.
//! An array is laid out element 0 highest, as a struct's first field
//! is, so the four lanes have the bits four named fields would have.
//! `Bright` takes pixels and puts out the brightest lane and its index,
//! going over the lanes in `for k in 0..4` and reading `p.lanes[k]`,
//! which the lowering folds into a slice of the port for each `k`. Its
//! netlist is checked against the run under nvc and Verilator.
use txhdl::comp::trace::{stop, Wave};
use txhdl::comp::{mux, signal, Clock, DefaultClock, In, Out, Running, Unit};
use txhdl::types::{Bit, U};
use txhdl::{lower, Trace, Value};

// begin{unit}
/// Four lanes of a byte, and whether the pixel is shown.
#[derive(Value, Clone, Copy, Default, Debug)]
pub struct Pixel {
    pub shown: Bit,
    pub lanes: [U<8>; 4],
}

/// The brightest lane of a pixel, and which it is: the first of equals.
#[derive(Trace, Default)]
pub struct Bright {}

#[lower]
impl Unit for Bright {
    async fn run(
        &mut self,
        px: In<Pixel>,
        (best, which): (Out<U<8>>, Out<U<2>>),
    ) {
        loop {
            DefaultClock::rising().await;
            let p = px.get();
            let mut b = p.lanes[0];
            let mut w = U::<2>::from(0u8);
            for k in 1..4 {
                let more = Bit::from(p.lanes[k] > b);
                b = mux(more, p.lanes[k], b);
                w = mux(more, U::<2>::from(k as u8), w);
            }
            best.set(mux(p.shown, b, U::<8>::from(0u8)));
            which.set(w);
        }
    }
}
// end{unit}

fn main() {
    let (px_out, px) = signal::<Pixel, DefaultClock>();
    let (best_out, best) = signal::<U<8>, DefaultClock>();
    let (which_out, which) = signal::<U<2>, DefaultClock>();
    let mut unit = Bright::default();
    if let Some(mut w) = Wave::from_env() {
        w.clock::<DefaultClock>();
        w.add("px", &px);
        w.add("best", &best);
        w.add("which", &which);
        w.start();
    }
    let mut sim = Running::new(unit.run(px, (best_out, which_out)));
    let pixels: [(bool, [u8; 4]); 4] = [
        (true, [10, 40, 20, 30]),
        (true, [90, 15, 90, 5]),
        (false, [1, 2, 3, 4]),
        (true, [0, 0, 0, 200]),
    ];
    let mut seen = Vec::new();
    for (shown, l) in pixels {
        px_out.set(Pixel {
            shown: Bit::from_bool(shown),
            lanes: l.map(U::from),
        });
        sim.cycle();
        seen.push((best.get().raw(), which.get().raw()));
    }
    println!("brightest and which: {seen:?}");
    assert_eq!(seen, vec![(40, 1), (90, 0), (0, 3), (200, 3)]);
    stop();
    let net = Bright::lowered("bright");
    txhdl::netlist::write_vhdl_from_env(&net);
    print!("\n{}", net.verilog());
}
