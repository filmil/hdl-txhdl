// SPDX-License-Identifier: Apache-2.0
//! The traced run of two lists: the small scene, then the next frame
//! drawn over it, written in once the rasteriser says the first is
//! done. It writes the trace and the rasteriser's netlist, so the
//! build checks the lowering across the hand-over from one list to
//! the next (issue 982), and prints both pictures.
use razboj::image;
use razboj::model;
use razboj::op;
use razboj::scene;
use razboj::sim;

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
    let first = op::assemble(&scene::small(), W, H);
    let second = op::assemble(&scene::small_next(), W, H);
    let runs = sim::run_lists::<LOGW, H, N, DL, CTRL>(
        &[first.clone(), second.clone()],
        true,
        true,
    );
    let both: Vec<_> = first.iter().chain(&second).copied().collect();
    assert_eq!(runs[0].fb, model::render(&first, W, H), "the first list");
    assert_eq!(runs[1].fb, model::render(&both, W, H), "the second list");
    for (i, run) in runs.iter().enumerate() {
        print!("{}", image::ascii(&run.fb, W, H));
        println!("list {}: done at cycle {}", i + 1, run.cycles);
    }
}
