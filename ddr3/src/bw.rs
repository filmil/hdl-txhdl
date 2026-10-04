// SPDX-License-Identifier: Apache-2.0
//! The bandwidth of the path into DDR3, measured in simulation: the
//! baseline issue 1023 asks for before the path is widened.
//!
//! Every user of the board's DDR3 reaches it through one path: the link
//! into `Ddr3Per`, its bridge `AxiWb`, and the Wishbone into the
//! controller's wrapper. The bridge takes one burst at a time and puts
//! each word on the Wishbone as its own request, waiting for the answer
//! before the next, so the path's throughput is one word per round trip
//! of the Wishbone, however many bursts the host has in flight; a
//! longer burst saves only the cycle or so a burst costs of its own.
//!
//! [`bridge`] measures that path with the controller replaced by a
//! Wishbone memory that answers after a given latency, so the cost per
//! word can be read off against the latency. [`ddr3_per`] measures the
//! peripheral itself, whose controller model answers after one cycle,
//! which is the best the path can do. The controller on the board takes
//! longer; the board run of issue 1023 measures how much.
use std::cell::Cell;
use std::rc::Rc;
use txhdl::comp::{join2, pad, signal, DefaultClock, Running, Unit};
use txhdl::types::{Bit, U};
use txhdl_parts::bus::axi::{
    axi_to_unit, AxiHost, AxiPer, HostLink, PerPort, Rd, Resp, Wr,
};
use txhdl_parts::bus::wb::sim::WbMem;
use txhdl_parts::bus::wb::{AxiWb, WbMaster};

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
/// before issuing past this; the bridge serves one at a time, so a few
/// is as good as many.
const WINDOW: usize = 4;

/// The host's side of a run: `bursts` bursts of `beats` words, all
/// reads or all writes, issued back to back with [`WINDOW`] in flight.
/// `done` is set when the last is answered.
async fn client(
    host: txhdl_parts::bus::axi::Host<32, 32, 4, 4, 16>,
    beats: usize,
    bursts: usize,
    write: bool,
    done: Rc<Cell<bool>>,
) {
    let data: Vec<U<32>> = (0..beats).map(|i| U::from(i as u32)).collect();
    let mut pending = std::collections::VecDeque::new();
    for b in 0..bursts {
        if pending.len() == WINDOW {
            let p: txhdl_parts::bus::axi::Pending<32, 4> =
                pending.pop_front().expect("a burst in flight");
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

/// The bridge in front of a Wishbone memory answering after `latency`
/// cycles: the cycles from the first burst issued to the last answered.
pub fn bridge(
    latency: u32,
    beats: usize,
    bursts: usize,
    write: bool,
) -> Measured {
    let HostLink {
        host,
        per_client,
        host_in,
        host_out,
        per_in,
        per_out,
    } = axi_to_unit::<32, 32, 4, 4, 16>();
    let bus = PerPort::from(per_client);
    let (cyc_o, cyc) = signal::<Bit, DefaultClock>();
    let (stb_o, stb) = signal::<Bit, DefaultClock>();
    let (we_o, we) = signal::<Bit, DefaultClock>();
    let (adr_o, adr) = signal::<U<28>, DefaultClock>();
    let (dat_o, dat) = signal::<U<32>, DefaultClock>();
    let (sel_o, sel) = signal::<U<4>, DefaultClock>();
    let (stall_o, stall) = signal::<Bit, DefaultClock>();
    let (ack_o, ack) = signal::<Bit, DefaultClock>();
    let (rdat_o, rdat) = signal::<U<32>, DefaultClock>();
    let mut h = AxiHost::<32, 32, 4, 4, 16>::default();
    let mut p = AxiPer::<32, 32, 4, 4>::default();
    let mut bridge = AxiWb::<32, 4, 28>::default();
    let mut mem = WbMem::<28>::new(latency, 0);
    let done = Rc::new(Cell::new(false));
    let finished = done.clone();
    let mut sim = Running::new(join2(
        join2(h.run(host_in, host_out), p.run(per_in, per_out)),
        join2(
            join2(
                mem.run(
                    (cyc, stb, we, adr, dat, sel),
                    (stall_o, ack_o, rdat_o),
                ),
                bridge.run(
                    bus,
                    WbMaster {
                        stall,
                        ack,
                        rdat,
                        cyc: cyc_o,
                        stb: stb_o,
                        we: we_o,
                        adr: adr_o,
                        dat: dat_o,
                        sel: sel_o,
                    },
                ),
            ),
            client(host, beats, bursts, write, done),
        ),
    ));
    let words = (beats * bursts) as u64;
    let cap = words * (latency as u64 + 64) + 1000;
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
    let HostLink {
        host,
        per_client,
        host_in,
        host_out,
        per_in,
        per_out,
    } = axi_to_unit::<32, 32, 4, 4, 16>();
    let bus: PerPort<32, 32, 4, 4> = per_client.into();
    let (_sys_clk_o, sys_clk) = signal::<Bit, DefaultClock>();
    let (_sys_rst_o, sys_rst) = signal::<Bit, DefaultClock>();
    let (calib_o, calib) = signal::<Bit, DefaultClock>();
    let bits = || signal::<Bit, DefaultClock>().0;
    let mut h = AxiHost::<32, 32, 4, 4, 16>::default();
    let mut p = AxiPer::<32, 32, 4, 4>::default();
    let mut mem = Ddr3Per::default();
    let done = Rc::new(Cell::new(false));
    let finished = done.clone();
    let mut sim = Running::new(join2(
        join2(h.run(host_in, host_out), p.run(per_in, per_out)),
        join2(
            mem.run(
                bus,
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

/// The shape the measurement found, held: through the bridge a word
/// costs the memory's latency and four cycles more, read or written,
/// since one word is in flight at a time, and a burst adds about one
/// cycle of its own, which is all a longer burst saves. When
/// issue 1023 widens the path this is the test that has to change.
#[cfg(test)]
mod tests {
    use super::{bridge, ddr3_per};

    #[test]
    fn a_word_costs_the_latency_and_four_cycles() {
        for latency in [1, 4, 16] {
            for write in [false, true] {
                let m = bridge(latency, 16, 16, write);
                let extra = m.cycles_per_word() - latency as f64;
                assert!(
                    (4.0..4.25).contains(&extra),
                    "latency {latency}, write {write}: {} cycles a word",
                    m.cycles_per_word()
                );
            }
        }
    }

    #[test]
    fn a_longer_burst_saves_only_its_own_cycle() {
        let short = bridge(8, 1, 64, false).cycles_per_word();
        let long = bridge(8, 64, 1, false).cycles_per_word();
        assert!(
            (0.75..1.25).contains(&(short - long)),
            "one-word bursts {short}, a 64-word burst {long}"
        );
    }

    #[test]
    fn the_peripheral_is_the_bridge_at_one_cycle() {
        let per = ddr3_per(16, 16, false).cycles_per_word();
        let best = bridge(1, 16, 16, false).cycles_per_word();
        assert!((per - best).abs() < 0.25, "Ddr3Per {per}, bridge {best}");
    }
}
