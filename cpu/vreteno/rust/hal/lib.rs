// SPDX-License-Identifier: Apache-2.0
//! What every program for Vreteno would otherwise write again.
//!
//! A program for this core used to start from a linker script, a
//! naked `_start` that set the stack and zeroed `.bss`, and raw
//! volatile writes at addresses copied from the board's map into each
//! program. That is the right amount of machinery for a greeting and
//! the wrong place to start the fourth program from. This crate holds
//! the machinery once:
//!
//! * [`map`], the address map of the board and the simulation, in one
//!   place;
//! * the peripherals as types with methods, [`Uart`], [`Timer`],
//!   [`Plic`], [`Pwm`] and [`Video`], each a set of volatile accesses
//!   at the map's addresses and nothing else;
//! * [`csr`], the control and status registers the core has;
//! * [`trap`], an entry that saves a frame, reads `mcause`, and
//!   dispatches an exception or an interrupt to a handler the program
//!   registered;
//! * [`entry!`], which owns the reset path, and [`halt`], which is how
//!   a program says it is done.
//!
//! Nothing here allocates, formats or unwinds: the crate is `no_std`,
//! the panic handler halts the core, and a program that links this
//! defines no panic handler of its own.
//!
//! The crate is inline RISC-V assembly from end to end and builds only
//! for the core, through the transition a program's image applies.
#![no_std]

use core::panic::PanicInfo;
use core::ptr::{read_volatile, write_volatile};

pub mod trap;

/// The address map: every base a program may touch, as the board's
/// router and the simulation's have them.
pub mod map {
    /// The boot memory, on the bus read-only: the program and its
    /// constants.
    pub const ROM: usize = 0x0000_0000;
    /// The data memory, 4 KiB.
    pub const DMEM: usize = 0x0000_1000;
    /// The core-local interrupt controller: the timer and the
    /// software interrupt, at the offsets every RISC-V platform has.
    pub const CLINT: usize = 0x0200_0000;
    /// The serial port.
    pub const UART: usize = 0x0000_3000;
    /// The pulse width modulator.
    pub const PWM: usize = 0x0000_3100;
    /// The video peripheral, on the flagship.
    pub const VIDEO: usize = 0x0000_3200;
    /// The remote peripheral, answered by a program across the
    /// Ethernet port.
    pub const REMOTE: usize = 0x0000_3300;
    /// The platform-level interrupt controller.
    pub const PLIC: usize = 0x0c00_0000;
    /// The DDR3 memory, where a loaded program lives.
    pub const DDR3: usize = 0x4000_0000;
}

/// A word read from a peripheral, every time it is asked for.
#[inline(always)]
fn rd(at: usize) -> u32 {
    // Every address in `map` is a device's register or a memory the
    // core reaches over the bus; reading it has no other effect.
    unsafe { read_volatile(at as *const u32) }
}

/// A word written to a peripheral, every time it is said.
#[inline(always)]
fn wr(at: usize, v: u32) {
    // As `rd`: a write to a device's register is the effect wanted.
    unsafe { write_volatile(at as *mut u32, v) }
}

/// The serial port: four words. The first is the byte to send, the
/// second the status, whose bit 0 is high while a frame is still going
/// out and whose bit 1 is high while a received byte waits, and the
/// third the oldest received byte, which reading takes.
pub struct Uart;

impl Uart {
    const DATA: usize = map::UART;
    const STATUS: usize = map::UART + 4;
    const RX: usize = map::UART + 8;
    const TX_BUSY: u32 = 1;
    const RX_READY: u32 = 2;

    /// Send one byte, once the port is free to take it. A byte written
    /// while the port is busy is dropped, so the wait is not optional.
    pub fn put(byte: u8) {
        while rd(Self::STATUS) & Self::TX_BUSY != 0 {}
        wr(Self::DATA, byte as u32);
    }

    /// Send every byte of `text`.
    pub fn say(text: &[u8]) {
        for byte in text {
            Self::put(*byte);
        }
    }

    /// Send a number as decimal digits, without division: the digits
    /// are found by subtraction from the largest power of ten, which
    /// keeps `core`'s formatting out of a program that cannot afford it.
    pub fn put_decimal(mut v: u32) {
        let mut place = 1_000_000_000u32;
        let mut started = false;
        while place > 0 {
            let mut digit = 0u8;
            while v >= place {
                v -= place;
                digit += 1;
            }
            if digit != 0 || started || place == 1 {
                Self::put(b'0' + digit);
                started = true;
            }
            place /= 10;
        }
    }

    /// Whether a received byte waits.
    pub fn ready() -> bool {
        rd(Self::STATUS) & Self::RX_READY != 0
    }

    /// The oldest received byte, if one waits.
    pub fn get() -> Option<u8> {
        if Self::ready() {
            Some(rd(Self::RX) as u8)
        } else {
            None
        }
    }

    /// The next received byte, waiting for it.
    pub fn take() -> u8 {
        loop {
            if let Some(byte) = Self::get() {
                return byte;
            }
        }
    }
}

/// The timer and the software interrupt: the count, which runs from
/// the reset one a cycle, the compare, and the software interrupt's
/// bit, at the offsets every RISC-V platform puts them at.
pub struct Timer;

impl Timer {
    const MSIP: usize = map::CLINT;
    const MTIMECMP: usize = map::CLINT + 0x4000;
    const MTIME: usize = map::CLINT + 0xbff8;

    /// The count, all 64 bits, read so that a carry between the two
    /// halves does not tear it.
    pub fn now() -> u64 {
        loop {
            let hi = rd(Self::MTIME + 4);
            let lo = rd(Self::MTIME);
            if rd(Self::MTIME + 4) == hi {
                return (hi as u64) << 32 | lo as u64;
            }
        }
    }

    /// The low half of the count, which wraps every 43 seconds at
    /// 100 MHz; enough for a wait, taken as a difference.
    pub fn ticks() -> u32 {
        rd(Self::MTIME)
    }

    /// Wait for `ticks` of the count.
    pub fn wait(ticks: u32) {
        let start = Self::ticks();
        while Self::ticks().wrapping_sub(start) < ticks {}
    }

    /// Raise the timer interrupt when the count reaches `when`. The
    /// high half goes first, set to the largest value, so that the
    /// compare never passes through a small value on the way.
    pub fn at(when: u64) {
        wr(Self::MTIMECMP + 4, u32::MAX);
        wr(Self::MTIMECMP, when as u32);
        wr(Self::MTIMECMP + 4, (when >> 32) as u32);
    }

    /// Raise or lower the software interrupt.
    pub fn soft(on: bool) {
        wr(Self::MSIP, on as u32);
    }
}

/// The platform-level interrupt controller, for one target: a
/// priority per source, the enable bits, the threshold, and the word
/// that claims on a read and completes on a write.
pub struct Plic;

impl Plic {
    const PRIORITY: usize = map::PLIC;
    const ENABLE: usize = map::PLIC + 0x2000;
    const THRESHOLD: usize = map::PLIC + 0x20_0000;
    const CLAIM: usize = map::PLIC + 0x20_0004;

    /// The serial port's receive interrupt.
    pub const UART: u32 = 1;
    /// The board's `irq` input.
    pub const BOARD: u32 = 2;

    /// Give `source` a priority and enable it. A priority of zero
    /// never interrupts.
    pub fn enable(source: u32, priority: u32) {
        wr(Self::PRIORITY + 4 * source as usize, priority);
        wr(Self::ENABLE, rd(Self::ENABLE) | 1 << source);
    }

    /// Disable `source`.
    pub fn disable(source: u32) {
        wr(Self::ENABLE, rd(Self::ENABLE) & !(1 << source));
    }

    /// Only a source with a priority above `t` interrupts.
    pub fn threshold(t: u32) {
        wr(Self::THRESHOLD, t);
    }

    /// The pending, enabled source of the highest priority, now being
    /// served; zero when there is none.
    pub fn claim() -> u32 {
        rd(Self::CLAIM)
    }

    /// Done serving `source`.
    pub fn complete(source: u32) {
        wr(Self::CLAIM, source);
    }
}

/// The pulse width modulator: a control word, the period, and a duty
/// per channel, in cycles.
pub struct Pwm;

impl Pwm {
    const CTRL: usize = map::PWM;
    const PERIOD: usize = map::PWM + 4;
    const DUTY: usize = map::PWM + 8;

    /// Run with `period` cycles a cycle, enabled, edge aligned, and
    /// every channel the right way up.
    pub fn run(period: u32) {
        wr(Self::PERIOD, period);
        wr(Self::CTRL, 1);
    }

    /// Channel `channel` high for `cycles` of each period.
    pub fn duty(channel: usize, cycles: u32) {
        wr(Self::DUTY + 4 * channel, cycles);
    }
}

/// The video peripheral: the status, whose bit 0 is high in the
/// vertical blanking and whose top half counts frames; the cursor, a
/// column and a row; and the pixel, which paints at the cursor and
/// moves it on.
pub struct Video;

impl Video {
    const STATUS: usize = map::VIDEO;
    const CURSOR: usize = map::VIDEO + 4;
    const PIXEL: usize = map::VIDEO + 8;

    /// Columns and rows of the framebuffer.
    pub const WIDTH: u32 = 160;
    pub const HEIGHT: u32 = 120;

    /// Whether the raster is in the vertical blanking.
    pub fn blanking() -> bool {
        rd(Self::STATUS) & 1 != 0
    }

    /// Frames shown, as a sixteen-bit count.
    pub fn frames() -> u32 {
        rd(Self::STATUS) >> 16
    }

    /// Put the cursor at a column and a row.
    pub fn cursor(x: u32, y: u32) {
        wr(Self::CURSOR, y << 8 | (x & 0xff));
    }

    /// Paint a twelve-bit colour at the cursor, and move it on.
    pub fn pixel(colour: u32) {
        wr(Self::PIXEL, colour);
    }
}

/// The control and status registers a program touches, each a read or
/// a write of the named register and nothing else. `mcause`, `mepc`
/// and the rest are reached through [`trap::Frame`] in a handler,
/// where they are already saved.
pub mod csr {
    /// The interrupt enable in `mstatus`.
    const MIE: u32 = 1 << 3;
    /// The timer interrupt's bit in `mie` and `mip`.
    pub const TIMER: u32 = 1 << 7;
    /// The software interrupt's bit.
    pub const SOFT: u32 = 1 << 3;
    /// The external interrupt's bit.
    pub const EXTERNAL: u32 = 1 << 11;

    /// Interrupts on.
    pub fn interrupts_on() {
        unsafe { core::arch::asm!("csrs mstatus, {0}", in(reg) MIE) }
    }

    /// Interrupts off.
    pub fn interrupts_off() {
        unsafe { core::arch::asm!("csrc mstatus, {0}", in(reg) MIE) }
    }

    /// Allow the interrupts in `bits`, of [`TIMER`], [`SOFT`] and
    /// [`EXTERNAL`].
    pub fn allow(bits: u32) {
        unsafe { core::arch::asm!("csrs mie, {0}", in(reg) bits) }
    }

    /// Refuse the interrupts in `bits`.
    pub fn refuse(bits: u32) {
        unsafe { core::arch::asm!("csrc mie, {0}", in(reg) bits) }
    }

    /// Clear the pending bits in `bits`. The external interrupt's is
    /// set by its line and cleared only this way, which [`super::trap`]
    /// does after its handler.
    pub fn clear_pending(bits: u32) {
        unsafe { core::arch::asm!("csrc mip, {0}", in(reg) bits) }
    }

    /// The cycle counter's low half.
    pub fn cycles() -> u32 {
        let v: u32;
        unsafe { core::arch::asm!("csrr {0}, mcycle", out(reg) v) }
        v
    }
}

/// Stop the core. A write of one to `mhalt` is how a program says it
/// is done; `ebreak` is a breakpoint and traps.
pub fn halt() -> ! {
    unsafe { core::arch::asm!("csrwi 0x7c0, 1", options(noreturn)) }
}

/// The reset path, once: `entry!(main)` defines `_start` at address
/// zero, which sets the stack pointer, zeroes the uninitialised data,
/// and jumps to `main`, which does not return. The three symbols come
/// from the linker script.
#[macro_export]
macro_rules! entry {
    ($main:path) => {
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
                main = sym $main,
                options(noreturn)
            )
        }
    };
}

/// Nothing can be reported and nothing can unwind, so a panic stops
/// the core the same way a finished program does.
#[panic_handler]
fn panic(_: &PanicInfo) -> ! {
    halt()
}
