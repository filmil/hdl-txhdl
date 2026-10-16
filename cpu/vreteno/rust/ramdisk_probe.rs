// SPDX-License-Identifier: Apache-2.0
//! The ramdisk of the boot image fastboot staged, read back where it
//! was staged (issue 1731): a program sent with `fastboot boot program
//! ramdisk` finds the ramdisk at `0x4800_0000` past the image's header
//! and the program's pages, as Doom finds its game data (issue 1177).
//!
//! It says where the ramdisk starts and how long it is, then its first
//! and last sixteen bytes as four little-endian words each. A host
//! compares those with the file it sent (`od -An -tx4`); the last words
//! are in the stage's last pages, so they show a large ramdisk staged
//! whole and readable.
//!
//! Nothing here divides: the page size is a power of two, and a header
//! whose page size is not is refused.
#![no_std]
#![no_main]

use core::ptr::read_volatile;
use vreteno_hal::{entry, halt, Uart};

/// Where fastboot stages a download: `fastboot_stage` in
/// `zephyr/fastboot/app/boards/ax7a200b.overlay`.
const STAGE: usize = 0x4800_0000;

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
    // the ramdisk's at 16 and the page size at 36.
    if word(STAGE) != 0x5244_4e41 || word(STAGE + 4) != 0x2144_494f {
        Uart::say(b"ramdisk: no boot image at the stage\n");
        halt()
    }
    let kernel = word(STAGE + 8) as usize;
    let size = word(STAGE + 16) as usize;
    let page = word(STAGE + 36) as usize;
    if page == 0 || page & (page - 1) != 0 || size < 16 {
        Uart::say(b"ramdisk: none, or a page size not a power of two\n");
        halt()
    }
    // The header's page, then the program's pages, then the ramdisk.
    let at = STAGE + page + ((kernel + page - 1) & !(page - 1));
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
