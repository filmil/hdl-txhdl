// SPDX-License-Identifier: Apache-2.0
//! The trap dispatcher at work: an environment call handled and
//! resumed past, and a timer interrupt taken, both through
//! `vreteno_hal::trap` rather than through a vector written by hand.
//!
//! The program registers a handler for `ecall` that counts the call
//! and moves `mepc` past it, makes three calls, and says how many it
//! counted. Then it registers a handler for the timer's line, asks the
//! timer for an interrupt a few hundred cycles on, turns interrupts on,
//! and waits in a loop that touches nothing until the handler has run
//! and disarmed the timer. It says what it saw and stops.
//!
//! Nothing here indexes a slice with a variable and nothing divides,
//! as in `hello.rs`, so nothing asks for `core`'s panic path; the
//! decimal digits come from the crate, by subtraction.
#![no_std]
#![no_main]

use core::ptr::{addr_of, addr_of_mut, read_volatile, write_volatile};
use vreteno_hal::trap::{self, Frame};
use vreteno_hal::{csr, entry, halt, Timer, Uart};

/// How many environment calls the program makes.
const CALLS: u32 = 3;
/// How far ahead the timer is set, in cycles.
const AHEAD: u64 = 400;

/// What the handlers counted, read in `main` with `read_volatile` so
/// the optimiser does not assume they never change.
static mut ECALLS: u32 = 0;
static mut TICKS: u32 = 0;

/// An environment call: count it and resume after it. `ecall` is a
/// thirty-two bit instruction, so after it is four bytes on.
fn on_ecall(frame: &mut Frame, _cause: u32) {
    unsafe {
        write_volatile(
            addr_of_mut!(ECALLS),
            read_volatile(addr_of!(ECALLS)) + 1,
        );
    }
    frame.mepc = frame.mepc.wrapping_add(4);
}

/// The timer: count it and disarm the timer, since its line stays
/// high while the count is past the compare.
fn on_timer(_frame: &mut Frame, _cause: u32) {
    unsafe {
        write_volatile(addr_of_mut!(TICKS), read_volatile(addr_of!(TICKS)) + 1);
    }
    Timer::at(u64::MAX);
}

entry!(main);

fn main() -> ! {
    trap::install();
    trap::on_exception(trap::ECALL, on_ecall);
    let mut i = 0;
    while i < CALLS {
        unsafe { core::arch::asm!("ecall") }
        i += 1;
    }
    Uart::say(b"ecall ");
    Uart::put_decimal(unsafe { read_volatile(addr_of!(ECALLS)) });

    trap::on_interrupt(trap::TIMER, on_timer);
    Timer::at(Timer::now() + AHEAD);
    csr::allow(csr::TIMER);
    csr::interrupts_on();
    while unsafe { read_volatile(addr_of!(TICKS)) } == 0 {}
    csr::interrupts_off();
    Uart::say(b" timer ");
    Uart::put_decimal(unsafe { read_volatile(addr_of!(TICKS)) });
    Uart::say(b"\n");
    halt()
}
