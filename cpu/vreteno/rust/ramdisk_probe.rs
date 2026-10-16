// SPDX-License-Identifier: Apache-2.0
//! The ramdisk of the boot image fastboot staged, read back where it
//! was staged (issue 1731): a program sent with `fastboot boot program
//! ramdisk` finds the ramdisk at `0x4800_0000` past the image's header
//! and the program's pages, as Doom finds its game data (issue 1177).
//!
//! It says where the ramdisk starts and how long it is, then its first
//! and last sixteen bytes as four little-endian words each. A host
//! compares those with the file it sent, `od -An -tx4 -N16 file` and
//! `tail -c 16 file | od -An -tx4`, the last sixteen bytes being at
//! any alignment; the last words are near the stage's end, so they
//! show a large ramdisk staged whole and readable.
//!
//! The header is trusted no further than the stage: a version other
//! than 0, a page size outside 2 KiB to 16 KiB or not a power of two,
//! or a ramdisk that would run past the stage, is refused. Nothing
//! here divides.
#![no_std]
#![no_main]

use core::ptr::read_volatile;
use vreteno_hal::{entry, halt, Uart};

/// Where fastboot stages a download, and where the stage ends:
/// `fastboot_stage` in `zephyr/fastboot/app/boards/ax7a200b.overlay`.
const STAGE: usize = 0x4800_0000;
const STAGE_END: usize = 0x5800_0000;

entry!(main);

fn word(at: usize) -> u32 {
    unsafe { read_volatile(at as *const u32) }
}

/// Four words from `at`, which need not be aligned, each little-endian.
fn say_words(at: usize) {
    for i in 0..4 {
        let mut w = 0u32;
        for b in 0..4 {
            let byte = unsafe { read_volatile((at + 4 * i + b) as *const u8) };
            w |= (byte as u32) << (8 * b);
        }
        Uart::put(b' ');
        Uart::put_hex(w);
    }
    Uart::put(b'\n');
}

fn main() -> ! {
    // The version 0 header: "ANDROID!", then the kernel's size at 8,
    // the ramdisk's at 16, the page size at 36 and the version at 40.
    if word(STAGE) != 0x5244_4e41 || word(STAGE + 4) != 0x2144_494f {
        Uart::say(b"ramdisk: no boot image at the stage\n");
        halt()
    }
    if word(STAGE + 40) != 0 {
        Uart::say(b"ramdisk: not a version 0 boot image\n");
        halt()
    }
    let kernel = word(STAGE + 8) as usize;
    let size = word(STAGE + 16) as usize;
    let page = word(STAGE + 36) as usize;
    if !(2048..=16384).contains(&page) || page & (page - 1) != 0 {
        Uart::say(b"ramdisk: a page size not a power of two in 2..16 KiB\n");
        halt()
    }
    // The header's page, then the program's pages, then the ramdisk,
    // all of it inside the stage; every sum is bounded by the stage, so
    // none of them wraps.
    let room = STAGE_END - STAGE;
    if kernel > room || size < 16 || size > room {
        Uart::say(b"ramdisk: none, or larger than the stage\n");
        halt()
    }
    let at = STAGE + page + ((kernel + page - 1) & !(page - 1));
    if at > STAGE_END || size > STAGE_END - at {
        Uart::say(b"ramdisk: runs past the stage\n");
        halt()
    }
    Uart::say(b"ramdisk at 0x");
    Uart::put_hex(at as u32);
    Uart::say(b" bytes ");
    Uart::put_decimal(size as u32);
    Uart::say(b" end 0x");
    Uart::put_hex((at + size) as u32);
    Uart::put(b'\n');
    Uart::say(b"ramdisk first");
    say_words(at);
    Uart::say(b"ramdisk last");
    say_words(at + size - 16);
    Uart::say(b"ramdisk done\n");
    halt()
}
