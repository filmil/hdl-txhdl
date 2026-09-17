// SPDX-License-Identifier: Apache-2.0
//! The HDMI demonstration's netlist: the video peripheral at 640 by 480
//! with a test picture in its framebuffer, and the I2C master timed for
//! the board, as Verilog on standard output.
//!
//! Nothing on the demonstration writes the framebuffer, so the picture
//! is in the netlist's initial values: eight colour bars over the top
//! two thirds, a grey ramp under them, and a white border.
use txhdl_parts::hdmi::{vga, Hdmi, I2cInit, FB_WORDS};

/// The framebuffer is 160 by 120, a framebuffer pixel 4 by 4.
type Video = Hdmi<
    { vga::HV },
    { vga::HFP },
    { vga::HSW },
    { vga::HBP },
    { vga::VV },
    { vga::VFP },
    { vga::VSW },
    { vga::VBP },
    2,
>;

/// A quarter of an I2C bit is 63 cycles of 25.2 MHz, so a bit is at
/// 100 kHz; the reset, and the wait after it, are 2 520 000 cycles,
/// a tenth of a second each.
type Master = I2cInit<63, 2_520_000>;

/// The colour of the test picture at a framebuffer pixel, as 12 bits.
fn picture(x: usize, y: usize) -> u128 {
    let (w, h) = (160, 120);
    if x == 0 || y == 0 || x == w - 1 || y == h - 1 {
        return 0xfff;
    }
    if y < 80 {
        // White, yellow, cyan, green, magenta, red, blue, black.
        const BARS: [u128; 8] =
            [0xfff, 0xff0, 0x0ff, 0x0f0, 0xf0f, 0xf00, 0x00f, 0x000];
        return BARS[x * 8 / w];
    }
    let g = (x * 16 / w) as u128;
    g << 8 | g << 4 | g
}

fn main() {
    let mut words = vec![0u128; FB_WORDS];
    for y in 0..120 {
        for x in 0..160 {
            words[y << 8 | x] = picture(x, y);
        }
    }
    let mut video = Video::lowered("hdmi_video");
    video.init("fb", &words);
    let master = Master::lowered("hdmi_i2c");
    print!("{}\n{}", video.verilog(), master.verilog());
}
