// SPDX-License-Identifier: Apache-2.0
//! The bandwidth of the path into DDR3, measured in simulation: what
//! issue 1023 asked of the path, before and after it was widened.
//!
//! Every user of the board's DDR3 reaches it through one path: the link
//! into `Ddr3Per`, its `AxiPerPins`, and the controller's own AXI4 port.
//! Nothing on the path holds a burst back: the pins part holds nothing,
//! and the controller takes several bursts at once and streams each
//! one's beats. So a read costs the controller's latency once a burst,
//! not once a word, and with a few bursts in flight even that overlaps,
//! which leaves a word a cycle. A write takes a beat a cycle too, with
//! four cycles between bursts that the host leaves (issue 1121).
//!
//! [`pins`] measures that path with the controller replaced by a memory
//! on its pins that answers after a given latency, so the cost per word
//! can be read off against the latency. [`ddr3_per`] measures the
//! peripheral itself, whose controller model answers after the
//! controller's latency as the vendor's simulation measured it. The
//! board run of issue 1023 measures the controller itself.
use std::cell::Cell;
use std::rc::Rc;
use txhdl::comp::{join2, pad, signal, DefaultClock, Running, Unit};
use txhdl::types::{Bit, U};
use txhdl_parts::bus::axi::{axi, AxiHost, Host, Link, Pending, Rd, Resp, Wr};
use txhdl_parts::bus::axi_per_pins::sim::pins as pin_ram;
use txhdl_parts::bus::axi_per_pins::AxiPerPins;

use crate::Ddr3Per;

/// What a run moved, and in how many cycles.
#[derive(Clone, Copy, Debug)]
pub struct Measured {
    pub words: u64,
    pub cycles: u64,
}

impl Measured {
    /// Cycles a word took, on average.
    pub fn cycles_per_word(&self) -> f64 {
        self.cycles as f64 / self.words as f64
    }
    /// Megabytes a second at a clock of `mhz`, four bytes a word.
    pub fn mb_per_s(&self, mhz: f64) -> f64 {
        4.0 * mhz / self.cycles_per_word()
    }
}

/// The first address the runs use, the board's DDR3 base.
const BASE: u32 = 0x4000_0000;

/// How many bursts the host keeps in flight. An identifier comes back
/// when its answer is taken, so the host waits for the oldest answer
/// before issuing past this.
pub const WINDOW: usize = 4;

/// The host's side of a run: `bursts` bursts of `beats` words, all
/// reads or all writes, issued back to back with [`WINDOW`] in flight.
/// `done` is set when the last is answered.
async fn client(
    host: Host<32, 32, 4, 5, 32>,
    beats: usize,
    bursts: usize,
    write: bool,
    done: Rc<Cell<bool>>,
) {
    let data: Vec<U<32>> = (0..beats).map(|i| U::from(i as u32)).collect();
    let mut pending = std::collections::VecDeque::new();
    for b in 0..bursts {
        if pending.len() == WINDOW {
            let p: Pending<32, 5> = pending.pop_front().expect("in flight");
            assert_eq!(p.done().await.resp, Resp::Okay);
        }
        let at = BASE + (b * beats * 4) as u32;
        let p = if write {
            host.write(Wr::at(at), &data).await
        } else {
            host.read(Rd::at(at, beats)).await
        };
        pending.push_back(p);
    }
    for p in pending {
        assert_eq!(p.done().await.resp, Resp::Okay);
    }
    done.set(true);
}

/// The pins part in front of a memory on its pins answering a read
/// `latency` cycles after taking its address, and a write `latency`
/// cycles after its last beat: the cycles from the first burst issued to the last
/// answered.
pub fn pins(
    latency: u64,
    beats: usize,
    bursts: usize,
    write: bool,
) -> Measured {
    let Link {
        host,
        host_in,
        host_out,
        per_in,
        per_out,
        ..
    } = axi::<32, 32, 4, 5, 32>();
    let (aw, ar, w, _, _) = per_in;
    let (_, _, b, r) = per_out;
    let (ram, inp, outp) = pin_ram::<32, 32, 4, 5>(aw, ar, w, b, r, 1 << 20);
    let ram = ram.wrapping().timed(latency, latency, 0);
    let mut h = AxiHost::<32, 32, 4, 5, 32>::default();
    let mut pinned = AxiPerPins::<32, 32, 4, 5>::default();
    let done = Rc::new(Cell::new(false));
    let finished = done.clone();
    let mut sim = Running::new(join2(
        h.run(host_in, host_out),
        join2(
            join2(ram.serve(), pinned.run(inp, outp)),
            client(host, beats, bursts, write, done),
        ),
    ));
    let words = (beats * bursts) as u64;
    let cap = words * (latency + 64) + 1000;
    let mut cycles = 0;
    while !finished.get() {
        sim.cycle();
        cycles += 1;
        assert!(cycles < cap, "the run did not finish in {cap} cycles");
    }
    Measured { words, cycles }
}

/// `Ddr3Per` itself, with its controller model: the cycles from the
/// controller's calibration to the last burst answered.
pub fn ddr3_per(beats: usize, bursts: usize, write: bool) -> Measured {
    let Link {
        host,
        host_in,
        host_out,
        per_in,
        per_out,
        ..
    } = axi::<32, 32, 4, 5, 32>();
    let (aw, ar, w, _, _) = per_in;
    let (_, _, b, r) = per_out;
    let (_sys_clk_o, sys_clk) = signal::<Bit, DefaultClock>();
    let (_sys_rst_o, sys_rst) = signal::<Bit, DefaultClock>();
    let (calib_o, calib) = signal::<Bit, DefaultClock>();
    let bits = || signal::<Bit, DefaultClock>().0;
    let mut h = AxiHost::<32, 32, 4, 5, 32>::default();
    let mut mem = Ddr3Per::default();
    let done = Rc::new(Cell::new(false));
    let finished = done.clone();
    let mut sim = Running::new(join2(
        h.run(host_in, host_out),
        join2(
            mem.run(
                (aw, ar, w, b, r),
                (
                    sys_clk,
                    sys_rst,
                    calib_o,
                    bits(),
                    bits(),
                    bits(),
                    bits(),
                    bits(),
                    bits(),
                    bits(),
                    bits(),
                    bits(),
                    bits(),
                    signal::<U<15>, DefaultClock>().0,
                    signal::<U<3>, DefaultClock>().0,
                    signal::<U<4>, DefaultClock>().0,
                    bits(),
                    pad::<U<32>, DefaultClock>(),
                    pad::<U<4>, DefaultClock>(),
                    pad::<U<4>, DefaultClock>(),
                ),
            ),
            client(host, beats, bursts, write, done),
        ),
    ));
    let words = (beats * bursts) as u64;
    let cap = words * 64 + 10_000;
    let mut cycles = 0u64;
    let mut from = None;
    while !finished.get() {
        sim.cycle();
        cycles += 1;
        if from.is_none() && calib.get().to_bool() {
            from = Some(cycles);
        }
        assert!(cycles < cap, "the run did not finish in {cap} cycles");
    }
    Measured {
        words,
        cycles: cycles - from.expect("the controller calibrated"),
    }
}

/// The shape the measurement found, held. When the path changes again
/// these are the tests that have to change.
#[cfg(test)]
mod tests {
    use super::{ddr3_per, pins, WINDOW};
    use crate::MODEL_READ_LATENCY;

    /// With bursts in flight, a long read streams a word a cycle
    /// whatever the latency, as long as the bursts in flight cover it.
    #[test]
    fn a_read_streams_a_word_a_cycle() {
        for latency in [1, 4, 16, 32] {
            let m = pins(latency, 16, 64, false);
            let c = m.cycles_per_word();
            assert!(
                (1.0..1.1).contains(&c),
                "latency {latency}: {c} cycles a word"
            );
        }
    }

    /// One burst at a time pays the latency once a burst: a word costs a
    /// cycle and the latency shared among the burst's words.
    #[test]
    fn one_burst_pays_the_latency_once() {
        for latency in [4u64, 16, 32] {
            let one = pins(latency, 16, 1, false).cycles_per_word();
            let none = pins(0, 16, 1, false).cycles_per_word();
            let extra = (one - none) * 16.0;
            assert!(
                (extra - latency as f64).abs() < 0.5,
                "latency {latency}: {extra} cycles more a burst"
            );
        }
    }

    /// Writes take a beat a cycle, and four cycles a burst more, whatever
    /// the latency: the pins part takes a beat in any step the link has
    /// one and the memory is ready, so those four are a gap on the
    /// link's write channel between one burst and the next, which is the
    /// host's, not the path's (issue 1121).
    #[test]
    fn a_write_takes_a_beat_a_cycle_and_four_a_burst() {
        for latency in [0, 16, 32] {
            // The last response's latency is the one nothing hides.
            let m = pins(latency, 16, 64, true);
            let c = (m.cycles - latency) as f64 / m.words as f64;
            assert!(
                (c - 20.0 / 16.0).abs() < 0.01,
                "latency {latency}: {c} cycles a word"
            );
        }
    }

    /// The peripheral with its model is the pins part at the model's
    /// latency, reads and writes alike, with the window large enough to
    /// cover it.
    #[test]
    fn the_peripheral_is_the_pins_at_the_models_latency() {
        assert!(WINDOW * 16 > MODEL_READ_LATENCY as usize);
        for write in [false, true] {
            let per = ddr3_per(16, 64, write).cycles_per_word();
            let at = pins(MODEL_READ_LATENCY as u64, 16, 64, write);
            let at = at.cycles_per_word();
            assert!((per - at).abs() < 0.1, "write {write}: {per} and {at}");
        }
    }
}
