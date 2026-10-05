// SPDX-License-Identifier: Apache-2.0
//! EGL's machine on the board, run by `board_test` through the board's
//! model (issue 996): what a swap does to the hardware, without the GL
//! library, which would not fit in the boot memory.
//!
//! The program writes one rectangle into Razboj's display list through
//! the machine, at the rows of the second buffer, has Razboj draw it,
//! points the scanout there and waits for the blanking, as a swap does.
//! Then it reads back two pixels, one inside the rectangle and one
//! beside it, and the scanout's base and show bit, and says `egl board
//! ok` or `egl board bad` with what it read.
#![no_std]
#![no_main]

use core::ptr::{read_volatile, write_volatile};
use gles_machine::Machine;
use gles_vreteno::Board;
use vreteno_hal::{entry, halt, Uart};

entry!(main);

/// The framebuffer, a row of 1024 words, and the second buffer's first
/// row.
const FRAME: usize = 0x4200_0000;
const ROW: usize = 1024;
const SECOND: u32 = 512;
/// The rectangle's colour as Razboj writes it, with its alpha.
const COLOUR: u32 = 0x00_5a_a5;

fn pixel(x: usize, y: usize) -> u32 {
    unsafe { read_volatile((FRAME as *const u32).add(y * ROW + x)) }
}

fn say_word(name: &[u8], v: u32) {
    Uart::say(name);
    Uart::put(b' ');
    Uart::put_hex(v);
    Uart::put(b'\n');
}

fn main() -> ! {
    let mut board = Board::new();
    // A rectangle, columns 10 to 13 of rows 512 to 514: kind 1, the
    // colour, its box, and an alpha of one. It is written through the
    // list's pointer, since indexing a slice would bring core's panic
    // path, which the boot memory has no room for.
    let entry = board.list().as_mut_ptr() as *mut u32;
    let words = [
        1 | (COLOUR << 2),
        10 | (SECOND << 16),
        13 | ((SECOND + 2) << 16),
    ];
    for k in 0..16 {
        let w = match k {
            0..=2 => words[k],
            15 => 0xff,
            _ => 0,
        };
        unsafe { write_volatile(entry.add(k), w) };
    }
    board.draw(1);
    board.show(SECOND);
    board.wait_blanking();

    let inside = pixel(11, 513);
    let beside = pixel(14, 513);
    let base = unsafe { read_volatile(0x3280 as *const u32) };
    let ctrl = unsafe { read_volatile(0x3284 as *const u32) };
    say_word(b"inside", inside);
    say_word(b"base", base);
    let ok = inside == 0xff00_0000 | COLOUR
        && beside != inside
        && base == FRAME as u32 + SECOND * 4096
        && ctrl & 1 == 1;
    Uart::say(if ok {
        b"egl board ok\n"
    } else {
        b"egl board bad\n"
    });
    halt()
}
