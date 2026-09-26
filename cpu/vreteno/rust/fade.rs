// SPDX-License-Identifier: Apache-2.0
//! A program that breathes an LED, to show the pulse width modulator
//! working on the board.
//!
//! The modulator shares the serial port's page: the port at `0x3000`
//! and the modulator at `0x3100`. Channel 0 drives the board's first
//! LED through the top level, and the program walks its duty up to the
//! period and back down, holding each step for a moment, so the light
//! rises and falls rather than stepping.
//!
//! The period is 10 000 cycles of the 100 MHz clock, which switches the
//! pin at 10 kHz: far above what an eye follows, so what is seen is the
//! average rather than a flicker. A step is held for about 4 ms, and
//! the walk up and back down takes about two seconds.
//!
//! The serial port says what the program is doing, once at the start
//! and then a dot per sweep, so a reader with no view of the board can
//! tell it is running. As in `hello.rs`, nothing here indexes a slice
//! and nothing divides, so nothing asks for `core`'s panic path.
#![no_std]
#![no_main]

use core::panic::PanicInfo;
use core::ptr::{read_volatile, write_volatile};

/// The serial port, as `hello.rs` has it.
const UART: *mut u32 = 0x3000 as *mut u32;
/// The timer's count, low half, one a cycle from the reset.
const MTIME: *mut u32 =
    (0x0200_0000 + vreteno_regs::timer::MTIME_LO) as *mut u32;
/// The modulator, on the same page as the serial port.
const PWM: *mut u32 = 0x3100 as *mut u32;
/// Its registers, as words from that base, as its map declares them
/// (issue 709).
const CTRL: usize = vreteno_regs::pwm::CTRL / 4;
const PERIOD: usize = vreteno_regs::pwm::PERIOD / 4;
const DUTY0: usize = vreteno_regs::pwm::DUTY0 / 4;

/// The period, in cycles of the 100 MHz clock: 10 kHz.
const PERIOD_CYCLES: u32 = 10_000;
/// How many steps the duty takes from dark to full.
const STEPS: u32 = 256;
/// How long a step is held, in cycles: about 4 ms.
const STEP_CYCLES: u32 = 400_000;

fn put(byte: u8) {
    unsafe {
        while UART.add(vreteno_regs::uart::STATUS / 4).read_volatile()
            & vreteno_regs::uart::STATUS_BUSY_MASK
            != 0
        {}
        write_volatile(UART.add(vreteno_regs::uart::TX / 4), byte as u32);
    }
}

fn say(line: &[u8]) {
    for byte in line {
        put(*byte);
    }
}

/// Waits for `ticks` of the timer's count. The count is 64 bits and
/// this reads the low half only, which wraps every 43 seconds at
/// 100 MHz, so the difference is taken as it wraps.
fn wait(ticks: u32) {
    let start = unsafe { read_volatile(MTIME) };
    while unsafe { read_volatile(MTIME) }.wrapping_sub(start) < ticks {}
}

/// Sets channel 0's duty, in cycles high per period.
fn duty(cycles: u32) {
    unsafe { write_volatile(PWM.add(DUTY0), cycles) };
}

#[no_mangle]
extern "C" fn main() -> ! {
    say(b"fade\n");
    unsafe {
        write_volatile(PWM.add(PERIOD), PERIOD_CYCLES);
        // Enabled, edge aligned, and every channel the right way up.
        write_volatile(PWM.add(CTRL), vreteno_regs::pwm::CTRL_ENABLE_MASK);
    }
    loop {
        let mut step = 0;
        while step < STEPS {
            duty(step * (PERIOD_CYCLES / STEPS));
            wait(STEP_CYCLES);
            step += 1;
        }
        while step > 0 {
            step -= 1;
            duty(step * (PERIOD_CYCLES / STEPS));
            wait(STEP_CYCLES);
        }
        say(b".");
    }
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
