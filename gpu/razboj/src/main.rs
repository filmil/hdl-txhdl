// SPDX-License-Identifier: Apache-2.0
//! The demonstration: render a scene on Razboj and show it.
//!
//! The picture is printed on the terminal in colour, and written as a
//! PNG where `RAZBOJ_PNG` says, which is how the document gets it. The
//! framebuffer is compared with the model as the run ends, so the
//! demonstration is a check as well as a picture.
use razboj::image;
use razboj::model;
use razboj::op;
use razboj::scene;
use razboj::sim;

/// The screen: sixty-four by sixty-four, one word a pixel.
const LOGW: usize = 6;
const W: usize = 1 << LOGW;
const H: usize = 64;
/// Words of memory: the framebuffer, then the display list and its
/// count, and a power of two.
const N: usize = 8192;
/// Where the display list sits, just above the framebuffer.
const DL: usize = 0x4000;
/// Where the count sits, above the longest list the scene makes.
const CTRL: usize = 0x4200;

fn main() {
    let ops = scene::house();
    let run = sim::run::<LOGW, H, N, DL, CTRL>(&ops, false, false);
    let want = model::render(&op::assemble(&ops, W, H), W, H);
    let wrong = run.fb.iter().zip(&want).filter(|(a, b)| a != b).count();
    assert_eq!(wrong, 0, "{wrong} pixels differ from the model");

    if std::env::args().any(|a| a == "--ascii") {
        print!("{}", image::ascii(&run.fb, W, H));
    } else {
        println!("{}", image::ansi(&run.fb, W, H));
    }
    println!(
        "{} entries, {W} by {H} pixels, {} cycles, every pixel as the \
         model says",
        ops.len(),
        run.cycles
    );
    if let Ok(p) = std::env::var("RAZBOJ_PNG") {
        std::fs::write(&p, image::png(&run.fb, W, H, 6)).expect("the PNG");
    }
}
