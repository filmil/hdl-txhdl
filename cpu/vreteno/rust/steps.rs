// SPDX-License-Identifier: Apache-2.0
//! The cycle counter read twice back to back, eight times, and each
//! difference printed: `mcycle step N` (issue 848).
//!
//! A read of `mcycle` once wrote the counter back, so every read lost a
//! count (issue 807). That is fixed and checked in simulation; this is
//! the program that shows it on the board. The two reads have to run
//! from memory whose fetch takes the same time every time, or a lost
//! count hides in the fetch: a loaded program runs from the DDR3, with
//! no cache, and a loop there moves by thirty cycles. So the program
//! writes the two reads, as four instructions, into the data memory,
//! which is block RAM, and calls them there. The core fetches over the
//! bus above its boot memory, so the data memory is code as well.
//!
//! Every line then says the same small number, the cycles from one
//! read to the next. `board_test` pins it, so a core that lost a count
//! per read again would print one fewer and fail there.
#![no_std]
#![no_main]

use core::ptr::{read_volatile, write_volatile};
use vreteno_hal::{entry, halt, map, Uart};

/// Where the routine goes: half way up the data memory, above anything
/// a program built into the boot memory keeps there and below its
/// stack, and unused by a loaded program, whose data and stack are in
/// the DDR3.
const AT: usize = map::DMEM + 0x800;

/// The routine, hand-encoded since it has to be data before it is
/// code: two reads of `mcycle`, their difference, and the return.
const ROUTINE: [u32; 4] = [
    0xb000_2573, // csrr a0, mcycle
    0xb000_25f3, // csrr a1, mcycle
    0x40a5_8533, // sub  a0, a1, a0
    0x0000_8067, // ret
];

/// How many differences the program prints.
const TIMES: u32 = 8;

entry!(main);

fn main() -> ! {
    let code = AT as *mut u32;
    for (i, w) in ROUTINE.iter().enumerate() {
        unsafe { write_volatile(code.add(i), *w) };
    }
    // The stores are posted; a fence waits for all of them to be
    // answered (issue 432), and there is no instruction cache, so the
    // routine is in place when it is fetched. Its last word read back
    // says so.
    unsafe { core::arch::asm!("fence") };
    if unsafe { read_volatile(code.add(ROUTINE.len() - 1)) } != ROUTINE[3] {
        Uart::say(b"mcycle routine not written\n");
        halt()
    }
    let step: extern "C" fn() -> u32 = unsafe { core::mem::transmute(AT) };
    for _ in 0..TIMES {
        let d = step();
        Uart::say(b"mcycle step ");
        Uart::put_decimal(d);
        Uart::put(b'\n');
    }
    halt()
}
