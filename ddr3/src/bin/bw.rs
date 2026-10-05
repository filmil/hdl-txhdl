// SPDX-License-Identifier: Apache-2.0
//! The path into DDR3, measured in simulation (issue 1023): the cost of
//! a word through the pins part against the latency of the controller
//! behind it, for long reads and long writes, with bursts in flight and
//! with one at a time, and the peripheral itself with its controller
//! model.
//!
//!   bazel run //ddr3:bw
use ddr3::bw::{ddr3_per, pins, WINDOW};

/// Bursts of sixteen words, sixty-four of them: 4 KiB a run.
const BEATS: usize = 16;
const BURSTS: usize = 64;
/// The board's bus clock.
const MHZ: f64 = 100.0;

fn main() {
    println!(
        "latency  reads, {WINDOW} in flight  MB/s   one at a time  MB/s   \
         writes  MB/s"
    );
    for latency in [1, 2, 4, 8, 16, 32] {
        let r = pins(latency, BEATS, BURSTS, false);
        let one = pins(latency, BEATS, 1, false);
        let w = pins(latency, BEATS, BURSTS, true);
        println!(
            "{latency:>7}  {:>18.2}  {:>5.1}  {:>13.2}  {:>5.1}  {:>7.2}  {:>5.1}",
            r.cycles_per_word(),
            r.mb_per_s(MHZ),
            one.cycles_per_word(),
            one.mb_per_s(MHZ),
            w.cycles_per_word(),
            w.mb_per_s(MHZ),
        );
    }
    let r = ddr3_per(BEATS, BURSTS, false);
    let w = ddr3_per(BEATS, BURSTS, true);
    println!(
        "Ddr3Per  {:>18.2}  {:>5.1}  {:>13}  {:>5}  {:>7.2}  {:>5.1}",
        r.cycles_per_word(),
        r.mb_per_s(MHZ),
        "",
        "",
        w.cycles_per_word(),
        w.mb_per_s(MHZ),
    );
}
