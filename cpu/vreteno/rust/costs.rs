// SPDX-License-Identifier: Apache-2.0
//! What an instruction costs the core on the board (issue 1561), for the
//! machine's timing mode: `board_test`'s `exclusive_costs_on_the_ddr3`
//! and `core_costs_for_the_machine`, which measure the same loops in
//! simulation, measured where the DDR3 is AMD's controller rather than
//! the simulation's model of it.
//!
//! Each kind is 256 of one instruction or pair in a loop, timed with
//! `mcycle` and less the empty loop. Every DDR3 word is in the DDR3 a
//! megabyte up, past a loaded program. Each line is `costs <kind>
//! <hundredths of a cycle an instruction>`; a host divides by a hundred.
#![no_std]
#![no_main]

use core::arch::asm;
use vreteno_hal::{entry, halt, map, Uart};

entry!(main);

/// A DDR3 word the loops use, and a stretch of 64 KiB for the loads that
/// miss, one line every 256 bytes.
const WORD: usize = map::DDR3 + 0x10_0000;
const STRETCH: usize = map::DDR3 + 0x20_0000;

/// Rounds of each loop.
const N: u32 = 256;

fn mcycle() -> u32 {
    let c: u32;
    unsafe { asm!("csrr {0}, mcycle", out(reg) c) };
    c
}

/// The cycles of `N` rounds of the loop whose body is `$body`, with `a0`
/// the word's address, `a1` the stretch's, `a2` and `a3` scratch, and a
/// fence after so that nothing is left on its way.
macro_rules! timed {
    ($($body:literal),*) => {{
        let t0 = mcycle();
        unsafe {
            asm!(
                ".option push",
                ".option arch, +a",
                "1:",
                $($body,)*
                "addi {n}, {n}, -1",
                "bnez {n}, 1b",
                "fence",
                ".option pop",
                n = inout(reg) N => _,
                in("a0") WORD,
                inout("a1") STRETCH => _,
                out("a2") _,
                out("a3") _,
            )
        };
        mcycle().wrapping_sub(t0)
    }};
}

fn say(name: &[u8], cycles: u32, empty: u32) {
    // Hundredths of a cycle an instruction: 100 / 256 is 25 / 64.
    let per = (cycles.wrapping_sub(empty) * 25) >> 6;
    Uart::say(b"costs ");
    Uart::say(name);
    Uart::put(b' ');
    Uart::put_decimal(per);
    Uart::put(b'\n');
}

fn main() -> ! {
    // The word in the data cache, and a divisor.
    unsafe { core::ptr::write_volatile(WORD as *mut u32, 7) };
    let _ = unsafe { core::ptr::read_volatile(WORD as *const u32) };
    let empty = timed!();
    say(b"load", timed!("lw a2, 0(a0)"), empty);
    say(b"store", timed!("sw a2, 0(a0)"), empty);
    say(b"amoadd.w", timed!("amoadd.w a2, a2, (a0)"), empty);
    say(b"lr.w+sc.w", timed!("lr.w a2, (a0)", "sc.w a3, a2, (a0)"), empty);
    say(b"lr.w", timed!("lr.w a2, (a0)"), empty);
    say(b"fence", timed!("fence"), empty);
    say(b"store+fence", timed!("sw a2, 0(a0)", "fence"), empty);
    say(b"load-miss+step", timed!("lw a2, 0(a1)", "addi a1, a1, 256"), empty);
    say(b"mul", timed!("mul a2, a2, a2"), empty);
    say(b"divu", timed!("lw a3, 0(a0)", "divu a2, a2, a3"), empty);
    Uart::say(b"costs done\n");
    halt()
}
