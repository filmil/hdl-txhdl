// SPDX-License-Identifier: Apache-2.0
//! A frame received by the Ethernet port, read back out of memory by
//! the core, as the Zephyr driver reads one.
//!
//! The frame comes in on the wire and the port's engines store it in
//! DDR3 without the core touching a byte of it (issue 151). This
//! program waits for the port to say a frame has arrived, reads its
//! length and slot from the port's registers, and then reads the frame
//! itself out of the slot in DDR3, through the same bus and the same
//! bridge the engines wrote it through. It says what it read on the
//! serial port and acknowledges the frame.
//!
//! So a run that says the right bytes has shown the whole receive
//! path, end to end: the wire, the split by EtherType, the length
//! found again after the crossing, the bytes packed into words, the
//! store engine's bursts, the arbiter, the router and the bridge into
//! the memory, and the register block's arrival. None of it is taken
//! on trust, because the bytes the core reads are the only evidence
//! offered, and they come out of the real memory.
//!
//! As in `hello.rs`, nothing here indexes a slice and nothing divides,
//! so nothing asks for `core`'s panic path.
#![no_std]
#![no_main]

use core::panic::PanicInfo;
use core::ptr::{read_volatile, write_volatile};

/// The serial port, as `hello.rs` has it.
const UART: *mut u32 = 0x3000 as *mut u32;
/// The Ethernet port's registers, on the fifth slot of the page.
const ETH: *mut u32 = 0x3400 as *mut u32;
/// Its words, as LiteEth lays them out.
const RX_SLOT: usize = 0;
const RX_LENGTH: usize = 1;
const RX_PENDING: usize = 2;
/// Where the receive slots begin, and how far apart they are.
const BUFS: u32 = 0x4100_0000;
const SLOT: u32 = 0x800;

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

/// One hexadecimal digit.
fn digit(n: u32) -> u8 {
    if n < 10 {
        b'0' + n as u8
    } else {
        b'a' + (n - 10) as u8
    }
}

/// The low `n` digits of `v`, most significant first.
fn hex(v: u32, n: u32) {
    let mut i = n;
    while i > 0 {
        i -= 1;
        put(digit((v >> (i * 4)) & 0xf));
    }
}

#[no_mangle]
extern "C" fn main() -> ! {
    // Wait for the port to say a frame is in memory. It says so when
    // the store engine goes idle, which is after the memory has
    // answered the frame's last write, so every read below finds the
    // frame already there.
    while unsafe { read_volatile(ETH.add(RX_PENDING)) } & 1 == 0 {}
    let len = unsafe { read_volatile(ETH.add(RX_LENGTH)) };
    let slot = unsafe { read_volatile(ETH.add(RX_SLOT)) } & 1;
    let base = (BUFS + slot * SLOT) as *const u32;

    say(b"rx ");
    hex(len, 4);
    put(b' ');
    // The frame a byte at a time, each byte taken out of its word
    // lowest lane first, which is the order the engines packed it in.
    let mut i = 0u32;
    while i < len {
        let w = unsafe { read_volatile(base.add((i >> 2) as usize)) };
        hex(w >> ((i & 3) * 8), 2);
        i += 1;
    }
    put(b'\n');

    // Acknowledge, as the driver does: the pending bit is write one to
    // clear.
    unsafe { write_volatile(ETH.add(RX_PENDING), 1) };
    unsafe { core::arch::asm!("csrwi 0x7c0, 1", options(noreturn)) }
}

/// The entry point, at address zero, which is where the core's
/// program counter starts, as `ddr3.rs` has it: the stack, then the
/// zeroed data, then `main`.
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
