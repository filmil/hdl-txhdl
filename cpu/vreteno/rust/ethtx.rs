// SPDX-License-Identifier: Apache-2.0
//! Two frames sent by the Ethernet port back to back, written into
//! memory by the core and fetched onto the wire by the port's engines,
//! as the Zephyr driver sends them.
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
//! wrong. Stores on this machine are posted, so before asking the port
//! to go, the program reads the frame's last word back. The link
//! answers a load behind an unanswered store to the same address with
//! the stored value, so once that read returns, the frame is in memory
//! for the engine to fetch. A `fence` would do the same since issue
//! 432 was fixed; the read-back is what was proven on the board and
//! stays.
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
/// The transmit slots: the slots begin at `0x4100_0000`, receive first,
/// and transmit 4096 bytes above them, 2048 bytes apart.
const TXBUF: u32 = 0x4100_1000;
const SLOT: u32 = 0x800;
/// Two frames. Neither length is a whole number of words: 23 leaves
/// three real bytes in its last word and 18 leaves two, so a byte side
/// that sent a whole last word would put a stray byte on the wire.
pub const LEN_A: u32 = 23;
pub const LEN_B: u32 = 18;

/// Byte `i` of frame `f`: an EtherType at bytes twelve and thirteen,
/// IPv4 for the first and ARP for the second, and every other byte
/// its own index over a mark that says which frame it belongs to.
fn byte(f: u32, i: u32) -> u32 {
    let len = if f == 0 { LEN_A } else { LEN_B };
    if i >= len {
        0
    } else if i == 12 {
        0x08
    } else if i == 13 {
        if f == 0 {
            0x00
        } else {
            0x06
        }
    } else if f == 0 {
        0x60 + i
    } else {
        0x90 + i
    }
}

/// Word `k` of frame `f`, lowest lane first.
fn word(f: u32, k: u32) -> u32 {
    let b = k * 4;
    byte(f, b)
        | byte(f, b + 1) << 8
        | byte(f, b + 2) << 16
        | byte(f, b + 3) << 24
}

/// Frame `f` written into slot `f`, and its last word read back so the
/// posted stores have landed before anybody is told to fetch it.
fn copy(f: u32) {
    let len = if f == 0 { LEN_A } else { LEN_B };
    let slot = (TXBUF + f * SLOT) as *mut u32;
    let words = (len + 3) >> 2;
    let mut k = 0u32;
    while k < words {
        unsafe { write_volatile(slot.add(k as usize), word(f, k)) };
        k += 1;
    }
    let _ = unsafe { read_volatile(slot.add((words - 1) as usize)) };
}

fn ready() -> bool {
    // Bound first: an `unsafe` block opening a function's body is a
    // statement, and the `& 1` after it would be a second expression.
    let r = unsafe { read_volatile(ETH.add(TX_READY)) };
    r & 1 == 1
}

/// The driver's send, in LiteEth's shape: wait until the port is ready,
/// then give it the slot, the length and the start, and return without
/// waiting for the frame to go.
fn send(slot: u32, len: u32) {
    while !ready() {}
    unsafe {
        write_volatile(ETH.add(TX_SLOT), slot);
        write_volatile(ETH.add(TX_LENGTH), len);
        write_volatile(ETH.add(TX_START), 1);
    }
}

#[no_mangle]
extern "C" fn main() -> ! {
    // Both frames are in memory before either is sent, so the two sends
    // follow each other with NOTHING between them. That is tighter than
    // a driver, which copies the next frame between sends: the second
    // send's wait for ready comes as soon after the first start as it
    // can, which is where reading a stale ready would bite. If this is
    // safe, the driver's shape, with more between, is safe with it.
    copy(0);
    copy(1);
    send(0, LEN_A);
    send(1, LEN_B);
    // Before halting, which ends the run, the second frame must have
    // gone. Busy first, then ready: the port stays busy through the
    // first frame, then holds `tx_go` for the second until it starts,
    // so ready reads one only once both are out. The second start lands
    // within a few cycles and the first frame takes far longer than
    // that, so it has landed by the time the first frame ends.
    while ready() {}
    while !ready() {}
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
