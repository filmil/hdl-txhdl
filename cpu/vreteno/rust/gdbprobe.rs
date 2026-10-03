// SPDX-License-Identifier: Apache-2.0
//! A program for gdb to load and stop (issue 872): it adds 0 to 9,
//! keeps the sum in `SUM`, and passes it to `reached`, a function of
//! its own name, where a breakpoint goes. At the breakpoint the sum is
//! 45 in `a0` and in `SUM`, which is what the test asks gdb for.
//!
//! It is linked for the DDR3, where gdb's `load` puts it, and prints
//! nothing, so it needs no serial port.
#![no_std]
#![no_main]

use core::ptr::{addr_of, addr_of_mut, read_volatile, write_volatile};
use vreteno_hal::{entry, halt};

/// The sum, where gdb can read it by name.
#[no_mangle]
pub static mut SUM: u32 = 0;

entry!(main);

/// Where the breakpoint goes; its argument is the sum. It reads `SUM`
/// back and returns how far the two differ, which is nothing; the read
/// is volatile, so the call is not optimised away.
#[no_mangle]
#[inline(never)]
pub extern "C" fn reached(sum: u32) -> u32 {
    unsafe { read_volatile(addr_of!(SUM)) }.wrapping_sub(sum)
}

fn main() -> ! {
    let mut sum = 0u32;
    for i in 0..10u32 {
        sum = sum.wrapping_add(core::hint::black_box(i));
    }
    unsafe { write_volatile(addr_of_mut!(SUM), sum) };
    core::hint::black_box(reached(sum));
    halt()
}
