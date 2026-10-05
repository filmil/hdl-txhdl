// SPDX-License-Identifier: Apache-2.0
//! Razboj on the board (issue 1169): the check that #985's rasteriser
//! draws on the flagship and that what it draws reaches the screen.
//!
//! The program shows the scanout from Razboj's frame, writes a display
//! list of a backdrop and three shapes, and rings the doorbell. It waits
//! for the count to come back to zero and for the rasteriser to say it
//! is idle, then reads pixels of each shape and of the backdrop back
//! over the bus. The picture stays on the screen.
//!
//! What it prints, a line each: `razboj probe`, `razboj rung`,
//! `razboj idle` and the cycles the list took, then `razboj ok`. A pixel
//! that is not what the list says prints `razboj bad`, its column, its
//! row and the word read; a list that is never finished prints
//! `razboj stuck` with the count and the status word.
#![no_std]
#![no_main]

mod razprobe_list;

use core::ptr::{read_volatile, write_volatile};
use razprobe_list::{CHECKS, ENTRIES, LIST, WORDS};
use vreteno_hal::{entry, halt, Razboj, Scan, Timer, Uart};

entry!(main);

/// One second of the timer.
const SECOND: u32 = 100_000_000;

/// The list, word by word, where the rasteriser reads it.
fn write_list() {
    let base = Razboj::LIST as *mut u32;
    let mut e = 0;
    while e < ENTRIES {
        let mut w = 0;
        while w < WORDS {
            unsafe { write_volatile(base.add(e * WORDS + w), LIST[e][w]) };
            w += 1;
        }
        e += 1;
    }
    // Read the last word back, so the posted stores have landed before
    // the count says the list is there.
    let _ = unsafe { read_volatile(base.add(ENTRIES * WORDS - 1)) };
}

/// The cycle counter's low word.
fn mcycle() -> u32 {
    let c: u32;
    unsafe { core::arch::asm!("csrr {0}, mcycle", out(reg) c) };
    c
}

fn main() -> ! {
    Uart::say(b"razboj probe\n");
    Scan::base(Razboj::FRAME);
    Scan::show(true);
    // A list may still be drawing from a run before this one.
    let start = Timer::ticks();
    while Razboj::count() != 0 && Timer::ticks().wrapping_sub(start) < SECOND
    {}
    write_list();
    let rung = mcycle();
    Razboj::ring(ENTRIES as u32);
    Uart::say(b"razboj rung\n");
    let start = Timer::ticks();
    loop {
        if Razboj::count() == 0 && Razboj::idle() {
            break;
        }
        if Timer::ticks().wrapping_sub(start) > SECOND {
            Uart::say(b"razboj stuck ");
            Uart::put_decimal(Razboj::count());
            Uart::put(b' ');
            Uart::put_decimal(Razboj::idle() as u32);
            Uart::put(b'\n');
            halt()
        }
    }
    let took = mcycle().wrapping_sub(rung);
    Uart::say(b"razboj idle ");
    Uart::put_decimal(took);
    Uart::put(b'\n');
    let frame = Razboj::FRAME as *const u32;
    let mut good = true;
    for (x, y, want) in CHECKS {
        let at = (y * (Scan::STRIDE / 4) + x) as usize;
        let got = unsafe { read_volatile(frame.add(at)) };
        if got != want {
            good = false;
            Uart::say(b"razboj bad ");
            Uart::put_decimal(x);
            Uart::put(b' ');
            Uart::put_decimal(y);
            Uart::put(b' ');
            Uart::put_hex(got);
            Uart::put(b'\n');
        }
    }
    if good {
        Uart::say(b"razboj ok\n");
    }
    halt()
}
