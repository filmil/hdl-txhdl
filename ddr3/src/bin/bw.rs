// SPDX-License-Identifier: Apache-2.0
//! The path into DDR3, measured in simulation (issue 1023): the cost of
//! a word through the bridge against the latency of the memory behind
//! it, for long reads and long writes, and the peripheral itself with
//! its controller model.
//!
//!   bazel run //ddr3:bw
use ddr3::bw::{bridge, ddr3_per};

/// Bursts of sixteen words, sixty-four of them: 4 KiB a run.
const BEATS: usize = 16;
const BURSTS: usize = 64;
/// The board's bus clock.
const MHZ: f64 = 100.0;

fn main() {
    println!("latency  reads: cycles/word  MB/s   writes: cycles/word  MB/s");
    for latency in [1, 2, 4, 8, 16, 32] {
        let r = bridge(latency, BEATS, BURSTS, false);
        let w = bridge(latency, BEATS, BURSTS, true);
        println!(
            "{latency:>7}  {:>19.2}  {:>5.1}  {:>20.2}  {:>5.1}",
            r.cycles_per_word(),
            r.mb_per_s(MHZ),
            w.cycles_per_word(),
            w.mb_per_s(MHZ),
        );
    }
    let r = ddr3_per(BEATS, BURSTS, false);
    let w = ddr3_per(BEATS, BURSTS, true);
    println!(
        "Ddr3Per  {:>19.2}  {:>5.1}  {:>20.2}  {:>5.1}",
        r.cycles_per_word(),
        r.mb_per_s(MHZ),
        w.cycles_per_word(),
        w.mb_per_s(MHZ),
    );
}
