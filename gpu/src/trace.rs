// SPDX-License-Identifier: Apache-2.0
//! The traced run: the same design on a screen small enough that a
//! waveform of the whole render can be drawn and a testbench
//! generated from it can be replayed. It writes the trace and the two
//! units' netlists, so the build checks the lowering against this
//! run, and prints the picture it drew.
use gpu::image;
use gpu::model;
use gpu::scene;
use gpu::sim;

/// The traced screen: sixteen by sixteen.
const LOGW: usize = 4;
const W: usize = 1 << LOGW;
const H: usize = 16;
const N: usize = 256;

fn main() {
    let ops = scene::small(W, H);
    let run = sim::run::<LOGW, H, N>(&ops, true, true);
    let want = model::render(&ops, W, H);
    assert_eq!(run.fb, want, "the hardware and the model differ");
    print!("{}", image::ascii(&run.fb, W, H));
    println!(
        "{} entries, {W} by {H} pixels, {} cycles",
        ops.len(),
        run.cycles
    );
}
