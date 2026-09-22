// SPDX-License-Identifier: Apache-2.0
//! A test of the board's DDR3 memory, run by the core: words written
//! across the first two gigabytes of the memory's region and read back,
//! and a line on the serial port that says whether every one came back.
//! The core halts when every word came back and spins when one did not,
//! so the board's first LED, which shows the halt, is the same verdict
//! for anybody who cannot read the serial port.
//!
//! Built with `--cfg=board_run`, for the board, the program also says a
//! header line before it touches the memory and a dot every ten seconds
//! before and after the test, so that a reader who attaches at any
//! moment can tell a silent serial path from a core that never ran.
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
/// The timer's count, low half, which runs one a cycle from the reset:
/// `CLINT_BASE + MTIME_OFF`, as `fade.rs` has it. It said `0x2000`
/// until issue 415, which is nobody's address: the router answers a
/// burst that matches no range with `DecErr`, and this program has no
/// trap handler and never sets `mtvec`, so the fault went to zero,
/// which is the boot memory, and restarted the loader. The dots that
/// exist to show the core is alive were what stopped it.
#[cfg(board_run)]
const MTIME: *mut u32 = 0x0200_bff8 as *mut u32;
/// Ten seconds of the board's 100 MHz clock.
#[cfg(board_run)]
const TEN_SECONDS: u32 = 1_000_000_000;
/// Dots before the test, and dots after it, one every ten seconds.
const DOTS_BEFORE: u32 = 3;
const DOTS_AFTER: u32 = 12;
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

/// Waits for `ticks` of the timer's count. The count is 64 bits and
/// this reads the low half only, which wraps every 43 seconds at 100
/// MHz, so the difference is taken as it wraps.
#[cfg(board_run)]
fn wait(ticks: u32) {
    let start = unsafe { read_volatile(MTIME) };
    while unsafe { read_volatile(MTIME) }.wrapping_sub(start) < ticks {}
}

/// Dots on the serial port, one every ten seconds. A reader who
/// attaches at any moment sees that the core is alive and that the
/// serial path carries bytes, whatever the test itself said.
#[cfg(board_run)]
fn dots(count: u32) {
    let mut i = 0;
    while i < count {
        wait(TEN_SECONDS);
        say(b".");
        i += 1;
    }
}

#[cfg(not(board_run))]
fn dots(_count: u32) {}

/// The line that says which program this is, before the memory is
/// touched. A simulation says nothing here: its serial port is read by
/// tests that state the whole of what the run said, and every extra
/// byte is simulated time they pay for.
#[cfg(board_run)]
fn header() {
    say(b"vreteno ddr3 test\n");
}

#[cfg(not(board_run))]
fn header() {}

/// The word index of the `i`th word, and the word that goes there.
fn at(i: usize) -> usize {
    i * STRIDE + i * 0x1_0101
}
fn word(i: usize) -> u32 {
    (at(i) as u32).rotate_left(7) ^ 0x5a5a_c3c3
}

#[no_mangle]
extern "C" fn main() -> ! {
    // The header and the dots go out before the memory is touched, so
    // a reader knows the core started and the serial path carries
    // bytes even when the test that follows says nothing. They are
    // there in the program built for the board, and not in the one a
    // simulation runs.
    header();
    dots(DOTS_BEFORE);
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
    dots(DOTS_AFTER);
    // The verdict on a board whose serial port cannot be read: the core
    // halts only when every word came back, so the first LED, which
    // shows the halt, says the test passed. A failure spins here
    // instead, and that LED stays dark. See issue #195.
    if bad != 0 {
        loop {}
    }
    unsafe { core::arch::asm!("csrwi 0x7c0, 1", options(noreturn)) }
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
    unsafe { core::arch::asm!("csrwi 0x7c0, 1", options(noreturn)) }
}
