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
//! * A frame that tests depth is binned into a megabyte two into the
//!   list's four, then laid out at the list as a tile table, its records
//!   first and its entries `razboj_tile::ENTRIES_AT` past them, and rung
//!   with the count's bit 31 set (#1273).
//! * The scanout's base, at `0x3280`, is the byte the buffer starts at,
//!   `0x4200_0000` and 4096 bytes a row; the first swap also sets the bit
//!   that shows the scanout.
//! * `glReadPixels` reads the framebuffer straight from the DDR3, both
//!   buffers' rows, once the frame so far is drawn (#999).
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
use razboj_tile::{ENTRIES_AT, TILE_WORDS, WORDS};
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
/// The framebuffer's rows the core may read back (#999): both buffers,
/// at rows 0 and 512, within four megabytes, short of the list.
const ROWS: usize = 1024;
/// The instructions a frame may hold: 256 KiB of the four megabytes the
/// board gives the list.
const ENTRIES: usize = 4096;
/// Room for a frame binned into tiles (#1273): a megabyte two into the
/// list's four, clear of the frame GL writes and of the tile table laid
/// out at the list.
const SCRATCH: usize = LIST + 0x20_0000;
const SCRATCH_SLOTS: usize = 16384;

fn rd(at: usize) -> u32 {
    // SAFETY: a register of the board's map.
    unsafe { read_volatile(at as *const u32) }
}

fn wr(at: usize, v: u32) {
    // SAFETY: a register of the board's map.
    unsafe { write_volatile(at as *mut u32, v) }
}

/// Rings the doorbell with `count` once the list's last word, at
/// `last`, reads back, so that every store before it has landed; then
/// waits until the list is drawn, the count back at zero and every
/// pixel written.
fn ring(last: usize, count: u32) {
    let (bell, status) =
        (DOORBELL + doorbell::COUNT, DOORBELL + doorbell::STATUS);
    let _ = rd(last);
    wr(bell, count);
    while rd(bell) & doorbell::COUNT_COUNT_MASK != 0
        || rd(status) & doorbell::STATUS_IDLE_MASK == 0
    {}
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
        while rd(DOORBELL + doorbell::COUNT) & doorbell::COUNT_COUNT_MASK != 0 {
        }
        ring(LIST + entries * WORDS * 4 - 4, entries as u32);
    }

    fn scratch(&mut self) -> &'static mut [[u32; WORDS]] {
        // SAFETY: the scratch room is EGL's alone, between a frame's
        // binning and its being laid out at the list.
        unsafe {
            core::slice::from_raw_parts_mut(
                SCRATCH as *mut [u32; WORDS],
                SCRATCH_SLOTS,
            )
        }
    }

    fn draw_tiled(
        &mut self,
        tiles: &[[u32; TILE_WORDS]],
        entries: &[[u32; WORDS]],
    ) {
        if tiles.is_empty() {
            return;
        }
        // The list is Razboj's until the last list is drawn.
        while rd(DOORBELL + doorbell::COUNT) & doorbell::COUNT_COUNT_MASK != 0 {
        }
        for (i, r) in tiles.iter().enumerate() {
            for (k, &w) in r.iter().enumerate() {
                wr(LIST + (i * TILE_WORDS + k) * 4, w);
            }
        }
        let at = LIST + ENTRIES_AT;
        for (i, e) in entries.iter().enumerate() {
            for (k, &w) in e.iter().enumerate() {
                wr(at + (i * WORDS + k) * 4, w);
            }
        }
        ring(
            at + entries.len() * WORDS * 4 - 4,
            tiles.len() as u32 | doorbell::COUNT_TILED_MASK,
        );
    }

    fn pixels(&mut self) -> Option<(&'static [u32], usize)> {
        let words = (STRIDE / 4) as usize;
        // SAFETY: the framebuffer is Razboj's, in the board's DDR3, and
        // the draw before a read has waited for every pixel; the core has
        // no data cache, so its loads see what Razboj wrote.
        let fb = unsafe {
            core::slice::from_raw_parts(
                FRAME as usize as *const u32,
                ROWS * words,
            )
        };
        Some((fb, words))
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
