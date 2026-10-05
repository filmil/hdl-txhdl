// SPDX-License-Identifier: Apache-2.0
//! A load the board refuses, said by the HAL's fault report (#1214):
//! the program reads a word from a hole in the board's map, with the
//! address in `a0`, and the report gives the cause, the instruction,
//! the address in `mtval`, and `a0` and `ra`. `board_test` runs it,
//! so that a board run of a program that reports its faults this way
//! is read knowing the report is right.
#![no_std]
#![no_main]

use vreteno_hal::{entry, halt, trap, Uart};

entry!(main);

/// Above the configuration flash and below the memory: no range of
/// `vreteno32::board::BoardMap` holds it, so the router answers it
/// with a decode error.
const HOLE: u32 = 0x3000_0000;

fn main() -> ! {
    trap::say_faults();
    Uart::say(b"fault say\n");
    unsafe {
        core::arch::asm!(
            "lw {0}, 0(a0)",
            out(reg) _,
            in("a0") HOLE,
        );
    }
    Uart::say(b"not refused\n");
    halt()
}
