// SPDX-License-Identifier: Apache-2.0
//! A frame sent by the Ethernet port, written into memory by the core
//! and fetched onto the wire by the port's engines, as the Zephyr
//! driver sends one.
//!
//! The core writes the frame into a transmit slot in DDR3, tells the
//! port which slot and how many bytes, and asks it to go. From there
//! the core touches nothing: the fetch engine reads the frame's words
//! back out of memory, `FrameOut` turns them into bytes, and the
//! sharing unit puts them on the wire. The test reads what left the
//! port and compares it with what the core wrote.
//!
//! One step here is the driver's obligation rather than the port's,
//! and it is in the program because a driver that left it out would be
//! wrong. Stores on this machine are posted and `fence` orders nothing
//! (issue 432), so before asking the port to go, the program reads the
//! frame's last word back. The link answers a load behind an unanswered
//! store to the same address with the stored value, so once that read
//! returns, the frame is in memory for the engine to fetch.
//!
//! As in `hello.rs`, nothing here indexes a slice and nothing divides,
//! so nothing asks for `core`'s panic path.
#![no_std]
#![no_main]

use core::panic::PanicInfo;
use core::ptr::{read_volatile, write_volatile};

/// The Ethernet port's registers, on the fifth slot of the page.
const ETH: *mut u32 = 0x3400 as *mut u32;
/// Its transmit words, as LiteEth lays them out.
const TX_SLOT: usize = 4;
const TX_LENGTH: usize = 5;
const TX_START: usize = 6;
const TX_READY: usize = 7;
/// Transmit slot zero: the slots begin at `0x4100_0000`, receive first,
/// and transmit 4096 bytes above them.
const TXBUF: *mut u32 = 0x4100_1000 as *mut u32;
/// The frame's length in bytes. Twenty three, so its last word holds
/// three real bytes and one that is not the frame's.
pub const LEN: u32 = 23;

/// Byte `i` of the frame: an IPv4 EtherType at bytes twelve and
/// thirteen, so the sharing unit has a type to look at, and every
/// other byte its own index over a mark.
fn byte(i: u32) -> u32 {
    if i == 12 {
        0x08
    } else if i == 13 {
        0x00
    } else if i < LEN {
        0x60 + i
    } else {
        0
    }
}

/// Word `k` of the frame, its bytes lowest lane first, which is the
/// order the fetch engine hands them to `FrameOut`.
fn word(k: u32) -> u32 {
    let b = k * 4;
    byte(b) | byte(b + 1) << 8 | byte(b + 2) << 16 | byte(b + 3) << 24
}

#[no_mangle]
extern "C" fn main() -> ! {
    let words = (LEN + 3) >> 2;
    let mut k = 0u32;
    while k < words {
        unsafe { write_volatile(TXBUF.add(k as usize), word(k)) };
        k += 1;
    }
    // The driver's obligation: the frame's last word read back, so the
    // posted stores have landed before the engine is told to fetch.
    let _ = unsafe { read_volatile(TXBUF.add((words - 1) as usize)) };

    unsafe {
        write_volatile(ETH.add(TX_SLOT), 0);
        write_volatile(ETH.add(TX_LENGTH), LEN);
        write_volatile(ETH.add(TX_START), 1);
    }
    // Busy first, then ready again. `tx_start` is a posted store, and
    // a load to another address may be answered before it lands, so
    // `tx_ready` can still read one just after the start was written:
    // reading one there says nothing about the frame, and waiting only
    // for one halted this program before the frame had left. So wait
    // for the port to take the frame, which is `tx_ready` going low,
    // and then for it to have sent it, which is `tx_ready` coming back.
    while unsafe { read_volatile(ETH.add(TX_READY)) } & 1 == 1 {}
    while unsafe { read_volatile(ETH.add(TX_READY)) } & 1 == 0 {}
    unsafe { core::arch::asm!("csrwi 0x7c0, 1", options(noreturn)) }
}

/// The entry point, at address zero, as `ddr3.rs` has it.
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
