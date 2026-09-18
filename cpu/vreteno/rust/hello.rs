// SPDX-License-Identifier: Apache-2.0
//! Hello world, in Rust, for Vreteno: a program compiled for the core
//! itself rather than for the machine the simulation runs on. It
//! writes a line to the serial port and stops.
//!
//! What the machine does not give you shapes the whole file. There is
//! no operating system and no standard library, so `no_std` and
//! `no_main`. Nothing sets the stack pointer, so the entry stub does.
//! Nothing zeroes `.bss`, so the entry stub does. A write of one to
//! `mhalt` halts the core, which is how a program exits; `ebreak` is a
//! breakpoint and traps. And the core cannot read its own instruction
//! memory, so every constant the program reads, this file's greeting
//! included, is linked into the data memory and carried in the image.
#![no_std]
#![no_main]

use core::panic::PanicInfo;
use core::ptr::write_volatile;

/// The serial port, four words at `0x3000`. The first is the byte to
/// send, the second the status, whose bit zero is high while a frame
/// is still going out.
const UART: *mut u32 = 0x3000 as *mut u32;

/// Send one byte, once the port is free to take it. A byte written
/// while the port is busy is dropped, so the wait is not optional.
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

/// What the program prints. A `static` rather than a literal in a
/// function, to say plainly that it is data and lives in the data
/// memory; a literal would too, but this is the thing being shown.
static GREETING: &[u8] = b"hello from rust\n";

/// The entry point, at address zero, which is where the core's
/// program counter starts. It is `.text.init` so that the linker puts
/// it first whatever the optimiser does with the rest.
#[no_mangle]
#[link_section = ".text.init"]
pub unsafe extern "C" fn _start() -> ! {
    core::arch::asm!(
        // The stack pointer, which nothing else sets.
        "la sp, __stack_top",
        // Zero the uninitialised data, which nothing else does.
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

#[no_mangle]
extern "C" fn main() -> ! {
    say(GREETING);
    // A write of one to `mhalt` stops this core. It is how a program
    // says it is done; `ebreak` is a breakpoint and traps.
    unsafe { core::arch::asm!("csrwi 0x7c0, 1", options(noreturn)) }
}

/// Nothing can be reported and nothing can unwind, so a panic stops
/// the machine the same way a finished program does.
#[panic_handler]
fn panic(_: &PanicInfo) -> ! {
    unsafe { core::arch::asm!("csrwi 0x7c0, 1", options(noreturn)) }
}
