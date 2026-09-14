// SPDX-License-Identifier: Apache-2.0
//! A foreign module as a unit, co-run with the Rust it came from. The
//! stage of `ex_stage` is lowered to Verilog and to VHDL by the build;
//! Verilator makes a C++ model of the Verilog and `verilog_unit()`
//! makes a unit of that, `stage_verilog::Stage`, and `vhdl_unit()`
//! makes one of the VHDL, `stage_vhdl::Stage`, with nvc running as a
//! child process behind it. Here the three sit in three pipelines,
//! fed the same offers, and the three sinks must take the same words
//! at the same ticks: the lowering checked live, side by side, in
//! both languages, rather than by replaying a trace. A module written
//! by hand goes in the same way.
use stage_verilog::Stage as StageV;
use stage_vhdl::Stage as StageH;
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
            out.send(v + 1);
            let count = self.count.get();
            self.count.set(count + 1);
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
            self.gap.set((gap.raw() as u8 + 1) % 2);
            if gap.raw() == 0 && out.ready().to_bool() {
                out.send(n);
                self.n.set(n + 1);
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
    let (e_tx, e_rx) = chan::<U<8>, _>();
    let (f_tx, f_rx) = chan::<U<8>, _>();
    let (mut source, mut stage, mut sink) =
        (Source::default(), Stage::default(), Sink::default());
    let (mut vsource, mut vstage, mut vsink) =
        (Source::default(), StageV::default(), Sink::default());
    let (mut hsource, mut hstage, mut hsink) =
        (Source::default(), StageH::default(), Sink::default());
    let (took, vtook, htook) =
        (sink.took.clone(), vsink.took.clone(), hsink.took.clone());
    if let Some(mut w) = Wave::from_env() {
        w.clock::<DefaultClock>();
        w.add("inp", &a_rx);
        w.add("out", &b_rx);
        w.add("stage", &stage);
        w.add("vinp", &c_rx);
        w.add("vout", &d_rx);
        w.add("hinp", &e_rx);
        w.add("hout", &f_rx);
        w.start();
    }
    // Three pipelines, the Rust stage, its Verilog and its VHDL; a
    // foreign unit before the sink that reads it, as any unit with
    // wires out.
    let mut sim = Running::new(join2(
        join2(
            join2(source.run((), a_tx), stage.run(a_rx, b_tx)),
            sink.run(b_rx, ()),
        ),
        join2(
            join2(
                join2(vsource.run((), c_tx), vstage.run(c_rx, d_tx)),
                vsink.run(d_rx, ()),
            ),
            join2(
                join2(hsource.run((), e_tx), hstage.run(e_rx, f_tx)),
                hsink.run(f_rx, ()),
            ),
        ),
    ));
    for _ in 0..12 {
        sim.cycle();
    }
    stop();
    let (took, vtook, htook) = (took.borrow(), vtook.borrow(), htook.borrow());
    for ((r, v), h) in took.iter().zip(vtook.iter()).zip(htook.iter()) {
        println!(
            "t={:>2} rust {:>3}   t={:>2} verilog {:>3}   t={:>2} vhdl {:>3}",
            r.0, r.1, v.0, v.1, h.0, h.1
        );
    }
    assert_eq!(*took, *vtook, "the Verilog stage and the Rust one differ");
    assert_eq!(*took, *htook, "the VHDL stage and the Rust one differ");
    println!("{} words, the same on all three sides", took.len());
}
