// SPDX-License-Identifier: Apache-2.0
//! The configuration flash read by the core (issue 312), in three
//! lines on the serial port, and nothing written to the chip.
//!
//! * `flash id` and the three bytes the chip answers to its identify
//!   command, `0x9f`, over the SPI master. The board's MT25QL128
//!   answers `0020ba18`, as eight digits, which is the smallest
//!   thing that shows the pins are the right pins.
//! * `flash wel` and the write enable latch, bit 1 of the chip's
//!   status register, read with `0x05` after the program has sent write
//!   enable, `0x06`. The pins refuse write enable, so the latch reads
//!   0; a 1 would say the chip took it, and could be erased.
//! * `flash sync at` and the byte offset of the bitstream's sync word,
//!   `aa 99 55 66`, in the first 256 bytes read through the window, or
//!   `flash no sync`. The flash holds the bitstream from its start, so
//!   the sync word is there on any board that booted from it, and
//!   finding it says the window reads the chip with no pattern written
//!   first.
#![no_std]
#![no_main]

use core::ptr::{read_volatile, write_volatile};
use vreteno_hal::{entry, halt, map, Uart};
use vreteno_regs::spi;

const SPI: *mut u32 = map::SPI as *mut u32;
const FLASH: *const u32 = map::FLASH as *const u32;

/// A half of a bit is four cycles, the least the pins allow.
const DIV: u32 = 3;
const HELD: u32 = DIV | spi::CTRL_SEL_MASK;
const FREE: u32 = DIV;

/// The sync word as the window reads it: the four bytes in flash
/// order, the lowest address in the lowest byte.
const SYNC: u32 = u32::from_le_bytes([0xaa, 0x99, 0x55, 0x66]);

entry!(main);

fn ctrl(v: u32) {
    unsafe { write_volatile(SPI.add(spi::CTRL / 4), v) }
}

/// One byte each way: sent, waited for, and what came back.
fn swap(byte: u32) -> u32 {
    unsafe {
        write_volatile(SPI.add(spi::DATA / 4), byte);
        while read_volatile(SPI.add(spi::STATE / 4)) & spi::STATE_BUSY_MASK != 0
        {
        }
        read_volatile(SPI.add(spi::DATA / 4)) & 0xff
    }
}

fn main() -> ! {
    // The identity: the command, then three bytes clocked back.
    ctrl(HELD);
    swap(0x9f);
    let mut id = 0;
    for _ in 0..3 {
        id = (id << 8) | swap(0);
    }
    ctrl(FREE);
    Uart::say(b"flash id ");
    Uart::put_hex(id);
    Uart::put(b'\n');

    // Write enable, which the pins refuse, then the status register.
    ctrl(HELD);
    swap(0x06);
    ctrl(FREE);
    ctrl(HELD);
    swap(0x05);
    let status = swap(0);
    ctrl(FREE);
    Uart::say(b"flash wel ");
    Uart::put_decimal((status >> 1) & 1);
    Uart::put(b'\n');

    // The window: the first 64 words, looking for the sync word.
    let mut at = None;
    for i in 0..64 {
        if unsafe { read_volatile(FLASH.add(i)) } == SYNC {
            at = Some(i as u32 * 4);
            break;
        }
    }
    match at {
        Some(off) => {
            Uart::say(b"flash sync at ");
            Uart::put_decimal(off);
            Uart::put(b'\n');
        }
        None => Uart::say(b"flash no sync\n"),
    }
    halt()
}
