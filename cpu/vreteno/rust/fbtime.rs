// SPDX-License-Identifier: Apache-2.0
//! What a full frame costs the core to update, the two ways the board
//! has (issue 151).
//!
//! The video peripheral's own framebuffer is 160 by 120 pixels of
//! twelve bits, and the core paints it a pixel at a time, a register
//! write each, through the cursor that moves on by itself. The scanout
//! shows 640 by 480 words of `0x00RRGGBB` from the DDR3, which the core
//! writes as memory and then shows with one write of the frame's base.
//!
//! The program times each with the cycle counter: the peripheral's
//! framebuffer painted whole, and a frame of the scanout painted whole
//! and switched to. It prints, a line each, the cycles, the cycles a
//! pixel in hundredths, and the cycles of a 640 by 480 screen at that
//! rate, so the two compare at the same size:
//!
//! ```text
//! fb regs <cycles> <hundredths a pixel> <a 640x480 screen>
//! fb ddr3 <cycles> <hundredths a pixel> <a 640x480 screen>
//! fb done
//! ```
#![no_std]
#![no_main]

use core::ptr::{read_volatile, write_volatile};
use vreteno_hal::{csr, entry, halt, Scan, Timer, Uart, Video};

entry!(main);

/// The frame, where scanprobe puts its own.
const FRAME: u32 = 0x4200_0000;
/// Words from one row of the frame to the next.
const ROW: u32 = Scan::STRIDE / 4;
/// The pixels of the screen the scanout shows.
const SCREEN: u32 = Scan::WIDTH * Scan::HEIGHT;

/// One line of the result: the cycles, the cycles a pixel in
/// hundredths, and what a screen of the scanout's size costs at that
/// rate.
fn say(what: &[u8], cycles: u32, pixels: u32) {
    Uart::say(what);
    Uart::put_decimal(cycles);
    Uart::put(b' ');
    Uart::put_decimal(((cycles as u64 * 100) / pixels as u64) as u32);
    Uart::put(b' ');
    Uart::put_decimal(((cycles as u64 * SCREEN as u64) / pixels as u64) as u32);
    Uart::put(b'\n');
}

/// The peripheral's framebuffer, painted whole, a register write a
/// pixel: a twelve-bit colour from the column and the row.
fn regs() -> u32 {
    let start = csr::cycles();
    Video::cursor(0, 0);
    let mut y = 0u32;
    while y < Video::HEIGHT {
        let mut x = 0u32;
        while x < Video::WIDTH {
            Video::pixel((x & 0xf) << 8 | (y & 0xf) << 4 | ((x ^ y) & 0xf));
            x += 1;
        }
        y += 1;
    }
    csr::cycles().wrapping_sub(start)
}

/// A frame of the scanout, painted whole into the DDR3, the last word
/// read back so the posted stores have landed, and switched to.
fn ddr3() -> u32 {
    let base = FRAME as *mut u32;
    let start = csr::cycles();
    let mut y = 0u32;
    while y < Scan::HEIGHT {
        let mut x = 0u32;
        while x < Scan::WIDTH {
            let at = (y * ROW + x) as usize;
            unsafe { write_volatile(base.add(at), x << 16 | y << 8 | (x ^ y)) };
            x += 1;
        }
        y += 1;
    }
    let last = ((Scan::HEIGHT - 1) * ROW + Scan::WIDTH - 1) as usize;
    let _ = unsafe { read_volatile(base.add(last)) };
    Scan::base(FRAME);
    Scan::show(true);
    csr::cycles().wrapping_sub(start)
}

fn main() -> ! {
    Uart::say(b"fb time\n");
    say(b"fb regs ", regs(), Video::WIDTH * Video::HEIGHT);
    say(b"fb ddr3 ", ddr3(), SCREEN);
    // The frame on the screen for a moment, for the eye.
    Timer::wait(100_000_000);
    Uart::say(b"fb done\n");
    halt();
}
