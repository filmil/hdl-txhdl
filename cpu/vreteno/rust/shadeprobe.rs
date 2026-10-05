// SPDX-License-Identifier: Apache-2.0
//! Gouraud shading on the board (issue 989): a triangle with red,
//! green and blue at its corners, drawn by Razboj on the flagship, and
//! every pixel of the screen read back by the core and held to what
//! the list says.
//!
//! The program shows the scanout from Razboj's frame, writes the list
//! of `shadeprobe_list.rs`, a backdrop and the triangle, rings the
//! doorbell and waits for the rasteriser to finish. Then it reads all
//! 640 by 480 pixels back over the bus and compares each with
//! `shadeprobe_list::expect`, which a host test holds to Razboj's model
//! pixel for pixel. The picture stays on the screen.
//!
//! What it prints, a line each: `shade probe`, `shade rung`,
//! `shade idle` and the cycles the list took, then `shade ok` and how
//! many pixels the triangle covers. A wrong pixel prints `shade bad`
//! with the first one's column, row, the word read and the word
//! expected, then how many were wrong; a list that is never finished
//! prints `shade stuck` with the count and the idle bit.
#![no_std]
#![no_main]

mod shadeprobe_list;

use core::ptr::{read_volatile, write_volatile};
use shadeprobe_list::{expect, BACK, ENTRIES, H, LIST, ROW, W, WORDS};
use vreteno_hal::{entry, halt, Razboj, Scan, Timer, Uart};

entry!(main);

/// One second of the timer.
const SECOND: u32 = 100_000_000;

/// The list, word by word, where the rasteriser reads it.
fn write_list() {
    let base = Razboj::LIST as *mut u32;
    for (e, entry) in LIST.iter().enumerate() {
        for (w, word) in entry.iter().enumerate() {
            unsafe { write_volatile(base.add(e * WORDS + w), *word) };
        }
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
    Uart::say(b"shade probe\n");
    Scan::base(Razboj::FRAME);
    Scan::show(true);
    // A list may still be drawing from a run before this one.
    let start = Timer::ticks();
    while Razboj::count() != 0 && Timer::ticks().wrapping_sub(start) < SECOND {}
    write_list();
    let rung = mcycle();
    Razboj::ring(ENTRIES as u32);
    Uart::say(b"shade rung\n");
    let start = Timer::ticks();
    while Razboj::count() != 0 || !Razboj::idle() {
        if Timer::ticks().wrapping_sub(start) > SECOND {
            Uart::say(b"shade stuck ");
            Uart::put_decimal(Razboj::count());
            Uart::put(b' ');
            Uart::put_decimal(Razboj::idle() as u32);
            Uart::put(b'\n');
            halt()
        }
    }
    let took = mcycle().wrapping_sub(rung);
    Uart::say(b"shade idle ");
    Uart::put_decimal(took);
    Uart::put(b'\n');

    let frame = Razboj::FRAME as *const u32;
    let (mut wrong, mut shaded) = (0u32, 0u32);
    for y in 0..H {
        for x in 0..W {
            let got =
                unsafe { read_volatile(frame.add((y * ROW + x) as usize)) };
            let want = expect(x, y);
            shaded += (want != BACK) as u32;
            if got != want {
                if wrong == 0 {
                    Uart::say(b"shade bad ");
                    Uart::put_decimal(x);
                    Uart::put(b' ');
                    Uart::put_decimal(y);
                    Uart::put(b' ');
                    Uart::put_hex(got);
                    Uart::put(b' ');
                    Uart::put_hex(want);
                    Uart::put(b'\n');
                }
                wrong += 1;
            }
        }
    }
    if wrong == 0 {
        Uart::say(b"shade ok ");
        Uart::put_decimal(shaded);
    } else {
        Uart::say(b"shade wrong ");
        Uart::put_decimal(wrong);
    }
    Uart::put(b'\n');
    halt()
}
