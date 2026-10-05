// SPDX-License-Identifier: Apache-2.0
//! EGL's machine on the board (issue 996): what a swap does to
//! Vreteno's peripherals, behind `gles_egl::Machine`.
//!
//! * The display list is Razboj's, in the DDR3 at `0x4280_0000`, and GL
//!   writes each frame straight into it. The core has no data cache, so
//!   the stores reach the memory in order; the last word is read back
//!   before the doorbell is rung, so that the posted stores have landed
//!   when Razboj reads the list, as `ico_hdmi.rs` does.
//! * The doorbell at `0x3900` takes the count, and reads zero again with
//!   the rasteriser's status idle once every pixel is written.
//! * The scanout's base, at `0x3280`, is the byte the buffer starts at,
//!   `0x4200_0000` and 4096 bytes a row; the first swap also sets the bit
//!   that shows the scanout.
//! * The vertical blanking is bit 0 of the video peripheral's status, at
//!   `0x3200`.
//!
//! The addresses are the HAL's `map` (`cpu/vreteno/rust/hal/lib.rs`),
//! stated here rather than taken from it, since the HAL brings a panic
//! handler of its own and a Zephyr program has another. Nothing here is
//! `#[no_mangle]`, so a program in the boot memory can use the machine
//! without EGL's entry points; `zephyr.rs` installs it for EGL.
#![no_std]

use core::ptr::{read_volatile, write_volatile};
use gles_machine::Machine;
use razboj_tile::WORDS;
use vreteno_regs::{doorbell, hdmi, scan};

/// The board's map, as `vreteno_hal::map` has it.
const VIDEO: usize = 0x0000_3200;
const SCAN: usize = 0x0000_3280;
const DOORBELL: usize = 0x0000_3900;
/// Razboj's framebuffer and display list, `vreteno32::board`'s
/// `RAZBOJ_FB` and `RAZBOJ_DL`.
const FRAME: u32 = 0x4200_0000;
const LIST: usize = 0x4280_0000;
/// Bytes from one row of the framebuffer to the next.
const STRIDE: u32 = 4096;
/// The instructions a frame may hold: 256 KiB of the four megabytes the
/// board gives the list.
const ENTRIES: usize = 4096;

fn rd(at: usize) -> u32 {
    // SAFETY: a register of the board's map.
    unsafe { read_volatile(at as *const u32) }
}

fn wr(at: usize, v: u32) {
    // SAFETY: a register of the board's map.
    unsafe { write_volatile(at as *mut u32, v) }
}

/// The board's machine: whether the scanout has been shown yet.
pub struct Board {
    shown: bool,
}

impl Board {
    /// A machine that has shown nothing yet.
    pub const fn new() -> Board {
        Board { shown: false }
    }
}

impl Default for Board {
    fn default() -> Self {
        Board::new()
    }
}

impl Machine for Board {
    fn list(&mut self) -> &'static mut [[u32; WORDS]] {
        // SAFETY: the list's memory is Razboj's alone, and EGL hands it
        // to GL and to Razboj in turn.
        unsafe {
            core::slice::from_raw_parts_mut(LIST as *mut [u32; WORDS], ENTRIES)
        }
    }

    fn draw(&mut self, entries: usize) {
        if entries == 0 {
            return;
        }
        let count = DOORBELL + doorbell::COUNT;
        let status = DOORBELL + doorbell::STATUS;
        while rd(count) & doorbell::COUNT_COUNT_MASK != 0 {}
        let _ = rd(LIST + entries * WORDS * 4 - 4);
        wr(count, entries as u32);
        while rd(count) & doorbell::COUNT_COUNT_MASK != 0
            || rd(status) & doorbell::STATUS_IDLE_MASK == 0
        {}
    }

    fn show(&mut self, row: u32) {
        wr(SCAN + scan::BASE, FRAME + row * STRIDE);
        if !self.shown {
            wr(SCAN + scan::CTRL, 1);
            self.shown = true;
        }
    }

    fn wait_blanking(&mut self) {
        let blank = || rd(VIDEO + hdmi::STATUS) & hdmi::STATUS_BLANK_MASK != 0;
        while blank() {}
        while !blank() {}
    }
}
