// SPDX-License-Identifier: Apache-2.0
//! The traced run: the same design on a screen small enough that a
//! waveform of the whole render can be drawn and a testbench
//! generated from it can be replayed. It writes the trace and the two
//! units' netlists, so the build checks the lowering against this
//! run, and prints the picture it drew.
use gpu::image;
use gpu::model;
use gpu::op;
use gpu::scene;
use gpu::sim;

/// The traced screen: sixteen by sixteen.
const LOGW: usize = 4;
const W: usize = 1 << LOGW;
const H: usize = 16;
/// Words of memory: the framebuffer, the display list and its count.
const N: usize = 1024;
/// Where the display list sits.
const DL: usize = 0x400;
/// Where the count sits.
const CTRL: usize = 0x600;

fn main() {
    let ops = scene::small();
    let run = sim::run::<LOGW, H, N, DL, CTRL>(&ops, true, true);
    let want = model::render(&op::assemble(&ops, W, H), W, H);
    assert_eq!(run.fb, want, "the hardware and the model differ");
    print!("{}", image::ascii(&run.fb, W, H));
    println!(
        "{} entries, {W} by {H} pixels, {} cycles",
        ops.len(),
        run.cycles
    );
}
