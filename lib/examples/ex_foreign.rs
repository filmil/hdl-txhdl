// SPDX-License-Identifier: Apache-2.0
//! A Verilog module as a unit, co-run with the Rust it came from. The
//! stage of `ex_stage` is lowered to Verilog by the build, Verilator
//! makes a C++ model of it, and `verilog_unit()` in the build makes a
//! unit of that: `stage_verilog::Stage`, with the same ports. Here it
//! sits in a second pipeline beside the Rust stage, fed the same
//! offers, and the two sinks must take the same words at the same
//! ticks: the lowering checked live, side by side, rather than by
//! replaying a trace. A module written by hand in Verilog goes in the
//! same way.
use stage_verilog::Stage as StageV;
use std::cell::RefCell;
use std::rc::Rc;
use txhdl::comp::trace::{stop, Wave};
use txhdl::comp::{
    chan, join2, now, until, Clock, DefaultClock, Reg, Running, Rx, Tx, Unit,
};
use txhdl::types::U;
use txhdl::Trace;

/// The stage, as `ex_stage` has it: a word taken and passed on plus
/// one whenever one is offered and there is room.
#[derive(Trace, Default)]
pub struct Stage {
    pub count: Reg<U<8>>,
}

impl Unit<Rx<U<8>>, Tx<U<8>>> for Stage {
    async fn run(&mut self, inp: Rx<U<8>>, out: Tx<U<8>>) {
        loop {
            until(DefaultClock::rising, || {
                inp.peek().is_some() && out.ready().to_bool()
            })
            .await;
            let v = inp.recv().unwrap_or_default();
            out.send(v.wrapping_add(1));
            let count = self.count.get();
            self.count.set(count.wrapping_add(1));
        }
    }
}

/// Offers a counting sequence every other cycle.
#[derive(Default)]
pub struct Source {
    pub n: Reg<U<8>>,
    pub gap: Reg<U<8>>,
}

impl Unit<(), Tx<U<8>>> for Source {
    async fn run(&mut self, _i: (), out: Tx<U<8>>) {
        loop {
            DefaultClock::rising().await;
            let (n, gap) = (self.n.get(), self.gap.get());
            self.gap.set(U::from((gap.raw() as u8 + 1) % 2));
            if gap.raw() == 0 && out.ready().to_bool() {
                out.send(n);
                self.n.set(n.wrapping_add(1));
            }
        }
    }
}

/// Takes every word and keeps it with the tick it came at.
#[derive(Default)]
pub struct Sink {
    pub took: Rc<RefCell<Vec<(u64, u8)>>>,
}

impl Unit<Rx<U<8>>, ()> for Sink {
    async fn run(&mut self, inp: Rx<U<8>>, _o: ()) {
        loop {
            let v = inp.wait().await;
            self.took.borrow_mut().push((now(), v.raw() as u8));
        }
    }
}

fn main() {
    let (a_tx, a_rx) = chan::<U<8>, _>();
    let (b_tx, b_rx) = chan::<U<8>, _>();
    let (c_tx, c_rx) = chan::<U<8>, _>();
    let (d_tx, d_rx) = chan::<U<8>, _>();
    let (mut source, mut stage, mut sink) =
        (Source::default(), Stage::default(), Sink::default());
    let (mut vsource, mut vstage, mut vsink) =
        (Source::default(), StageV::default(), Sink::default());
    let (took, vtook) = (sink.took.clone(), vsink.took.clone());
    if let Some(mut w) = Wave::from_env() {
        w.clock::<DefaultClock>();
        w.add("inp", &a_rx);
        w.add("out", &b_rx);
        w.add("stage", &stage);
        w.add("vinp", &c_rx);
        w.add("vout", &d_rx);
        w.start();
    }
    // Two pipelines, the Rust stage in one and its Verilog in the
    // other; the foreign unit before the sink that reads it, as any
    // unit with wires out.
    let mut sim = Running::new(join2(
        join2(
            join2(source.run((), a_tx), stage.run(a_rx, b_tx)),
            sink.run(b_rx, ()),
        ),
        join2(
            join2(vsource.run((), c_tx), vstage.run(c_rx, d_tx)),
            vsink.run(d_rx, ()),
        ),
    ));
    for _ in 0..12 {
        sim.cycle();
    }
    stop();
    let (took, vtook) = (took.borrow(), vtook.borrow());
    for (r, v) in took.iter().zip(vtook.iter()) {
        println!(
            "t={:>2} rust took {:>3}    t={:>2} verilog took {:>3}",
            r.0, r.1, v.0, v.1
        );
    }
    assert_eq!(*took, *vtook, "the Verilog stage and the Rust one differ");
    println!("{} words, the same on both sides", took.len());
}
