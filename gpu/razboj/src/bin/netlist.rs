// SPDX-License-Identifier: Apache-2.0
//! Razboj's rasteriser as the board would have it, as Verilog on
//! standard output, for synthesis out of context (issue 984).
//!
//! The parameters are the board's: thirty-two-bit addresses and the
//! two-bit identifiers its host ports carry, and a 640 by 480 screen
//! in rows of 1024 pixels, the scanout's mode with a row a power of two
//! wide. The three addresses are in the board's DDR3, which starts at
//! 0x4000_0000, and stand in for the ones the flagship will give it
//! (issue 985): they are constants the logic adds, so their values move
//! a few gates and not the size or the timing.
use razboj::raster::Raster;

/// Address bits, as on the board's bus.
const A: usize = 32;
/// Identifier bits, as the board's host ports carry.
const I: usize = 2;
/// The screen: rows of `1 << LOGW` pixels, `H` of them.
const LOGW: usize = 10;
const H: usize = 480;
/// The framebuffer, the display list and its count, in DDR3. The list
/// has room for 65535 entries of thirty-two bytes before the count.
const BASE: usize = 0x4400_0000;
const DL: usize = 0x4300_0000;
const CTRL: usize = 0x4320_0000;

fn main() {
    let net = Raster::<A, I, LOGW, H, BASE, DL, CTRL>::lowered("razboj");
    print!("{}", net.verilog());
}
