// SPDX-License-Identifier: Apache-2.0
//! A test of the board's DDR3 memory, run by the core: words written
//! across the first two gigabytes of the memory's region and read back,
//! and a line on the serial port that says whether every one came back.
//!
//! The words go at addresses a sixteenth of the region apart, each at a
//! different offset into its sixteenth, so a mistake in the high address
//! bits and a mistake in the low ones both show. Each word is its own
//! address mixed with a constant, so no two are alike and a word read
//! from the wrong place is a wrong word. The whole of the pattern is
//! written before any of it is read, so a write that lands on another
//! word's address shows too.
//!
//! As in `hello.rs`, nothing here indexes a slice and nothing divides,
//! so nothing asks for `core`'s panic path.
#![no_std]
#![no_main]

use core::panic::PanicInfo;
use core::ptr::{read_volatile, write_volatile};

/// The serial port, as `hello.rs` has it.
const UART: *mut u32 = 0x3000 as *mut u32;
/// The memory's region: a gigabyte of words from here.
const DDR3: *mut u32 = 0x4000_0000 as *mut u32;
/// How many words the test writes, and how far apart, in words.
const COUNT: usize = 16;
const STRIDE: usize = 1 << 24;

fn put(byte: u8) {
    unsafe {
        while UART.add(1).read_volatile() & 1 != 0 {}
        write_volatile(UART, byte as u32);
    }
}

fn say(line: &[u8]) {
    for byte in line {
        put(*byte);
    }
}

/// The word index of the `i`th word, and the word that goes there.
fn at(i: usize) -> usize {
    i * STRIDE + i * 0x1_0101
}
fn word(i: usize) -> u32 {
    (at(i) as u32).rotate_left(7) ^ 0x5a5a_c3c3
}

#[no_mangle]
extern "C" fn main() -> ! {
    say(b"ddr3 ");
    for i in 0..COUNT {
        unsafe { write_volatile(DDR3.add(at(i)), word(i)) };
    }
    let mut bad = 0u32;
    for i in 0..COUNT {
        if unsafe { read_volatile(DDR3.add(at(i))) } != word(i) {
            bad += 1;
        }
    }
    say(if bad == 0 { b"ok\n" } else { b"bad\n" });
    unsafe { core::arch::asm!("ebreak", options(noreturn)) }
}

/// The entry point, at address zero, which is where the core's
/// program counter starts.
#[no_mangle]
#[link_section = ".text.init"]
pub unsafe extern "C" fn _start() -> ! {
    core::arch::asm!(
        "la sp, __stack_top",
        "la t0, __bss_start",
        "la t1, __bss_end",
        "1:",
        "beq t0, t1, 2f",
        "sw zero, 0(t0)",
        "addi t0, t0, 4",
        "j 1b",
        "2:",
        "j {main}",
        main = sym main,
        options(noreturn)
    )
}

/// Nothing can be reported and nothing can unwind, so a panic stops
/// the machine the same way a finished program does.
#[panic_handler]
fn panic(_: &PanicInfo) -> ! {
    unsafe { core::arch::asm!("ebreak", options(noreturn)) }
}
