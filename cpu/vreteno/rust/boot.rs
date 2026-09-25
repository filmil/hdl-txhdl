// SPDX-License-Identifier: Apache-2.0
//! A loader that lives in the boot memory and takes a program off the
//! serial port.
//!
//! Until the core could fetch from the bus, a program lived in the
//! netlist and changing it meant a place and route. The core fetches
//! from the bus above the boot memory now, so a program can be written
//! into the board's DDR3 and jumped to, and the loop becomes: build an
//! image, send it down the wire, watch it run.
//!
//! ## The stream
//!
//! Deliberately dumb, as issue 135 asks. Every number is four bytes,
//! least significant first, which is how the core reads memory anyway.
//!
//! | Field | What it is |
//! |---|---|
//! | magic | `TXLD`, so noise on the line is not mistaken for a load |
//! | address | where the bytes go, and where the loader jumps |
//! | length | how many bytes follow, a multiple of four |
//! | bytes | the program, length of them |
//! | checksum | the sum of the words above, wrapping |
//!
//! The loader types `K` when it has taken the header and after every
//! block of `BLOCK_WORDS` words, and the sender waits for it. Without
//! that the sender has to know how fast the loader writes: the port
//! buffers eight bytes, a write into memory takes longer than a byte
//! takes to arrive at any rate worth using, and the difference is a
//! stream that is silently short. One byte per block costs nothing and
//! removes the question.
//!
//! The address is in the stream rather than assumed, because assuming
//! it is the kind of decision that is cheap now and awkward later.
//!
//! ## What it says
//!
//! Every step says something on the serial port, so a load that failed
//! is never confused with a program that loaded and then crashed:
//! `boot` when it is waiting, `load` when a header arrived, `ok` before
//! it jumps, and `bad magic`, `bad len` or `bad sum` when it will not.
//! After a refusal it waits for another stream rather than stopping,
//! since the usual cause is a half-typed command on the other end.
//! Two more words tell a loaded program that came back from a reset,
//! which read the same on the line otherwise (issue 413). A trap taken
//! before the program set `mtvec` lands at address zero, where the
//! loader starts over; that pass says `trap`, the cause and the
//! address first, since a trap fills `mcause` and a start from
//! configuration finds it clear. On the board, a one-word `ebreak`
//! image says `trap 00000003 at 40000000`, and a one-word `ret` says
//! `trap 00000002 at` the word after the loader's jump: `entry` never
//! returns as far as the compiler knows, so nothing runnable follows
//! the jump, and a program that returns reads as that illegal word
//! rather than as the loop going round. Every pass of the loop after
//! the first therefore says `boot again` only after a refusal, and a
//! reset, which starts the loop from nothing, cannot say it.
//!
//! As in `hello.rs`, nothing here indexes a slice and nothing divides,
//! so nothing asks for `core`'s panic path.
#![no_std]
#![no_main]

use core::panic::PanicInfo;
use core::ptr::write_volatile;

/// The serial port, as `hello.rs` has it: the data word, then the
/// status, whose bit 0 is busy sending and bit 1 a byte waiting.
const UART: *mut u32 = 0x3000 as *mut u32;
const UART_STATUS: usize = 1;
const UART_RX: usize = 2;
const TX_BUSY: u32 = 1;
const RX_READY: u32 = 2;

/// `TXLD`, least significant byte first.
const MAGIC: u32 = 0x444c_5854;
/// The memory a program may be loaded into: the quarter of the address
/// space the router gives the board's DDR3.
const RAM_BASE: u32 = 0x4000_0000;
const RAM_MASK: u32 = 0xc000_0000;
/// As much as this loader will take, which is far more than the board
/// has room to run and far less than the memory holds.
const MAX_BYTES: u32 = 64 * 1024 * 1024;
/// How many words the loader takes before it asks for more.
///
/// One. The port buffers eight bytes, and a sender that is a block
/// ahead has that block outstanding plus whatever the acknowledgement
/// before it has not yet freed; two words was measured to lose the end
/// of a stream, and one word does not. It costs a byte of line for
/// every four, which a loader can afford, and it means the sender
/// never has to know how fast the memory it writes to answers.
const BLOCK_WORDS: u32 = 1;
/// What it says when it is ready for the next block.
const ACK: u8 = b'K';

fn put(byte: u8) {
    unsafe {
        while UART.add(UART_STATUS).read_volatile() & TX_BUSY != 0 {}
        write_volatile(UART, byte as u32);
    }
}

fn say(line: &[u8]) {
    for byte in line {
        put(*byte);
    }
}

/// The next byte on the line, waiting for as long as it takes.
fn get() -> u32 {
    unsafe {
        while UART.add(UART_STATUS).read_volatile() & RX_READY == 0 {}
        UART.add(UART_RX).read_volatile() & 0xff
    }
}

/// The next four bytes as a word, least significant first.
fn get_word() -> u32 {
    let a = get();
    let b = get();
    let c = get();
    let d = get();
    a | (b << 8) | (c << 16) | (d << 24)
}

/// Says a word as eight hexadecimal digits, for a message that has to
/// carry a number.
fn say_hex(word: u32) {
    let digits = b"0123456789abcdef";
    let mut shift = 28;
    loop {
        let nibble = ((word >> shift) & 0xf) as usize;
        put(digits[nibble]);
        if shift == 0 {
            return;
        }
        shift -= 4;
    }
}

/// Whether an address is inside the memory a program may be loaded
/// into. A stream that names anywhere else is refused rather than
/// allowed to write over the peripherals.
fn in_ram(addr: u32, len: u32) -> bool {
    if addr & RAM_MASK != RAM_BASE {
        return false;
    }
    let end = addr.wrapping_add(len);
    end > addr && (end - 1) & RAM_MASK == RAM_BASE
}

#[no_mangle]
extern "C" fn main() -> ! {
    // The loader never sets `mtvec`, so a trap taken by a loaded
    // program before it has set its own comes to address zero, which
    // is `_start`, and the loader runs again as if from reset. A reset
    // leaves `mcause` and `mepc` zero and a trap does not, so the
    // loader says which it was, with the cause and the address, before
    // the greeting (issue 413).
    let (mcause, mepc): (u32, u32);
    unsafe {
        core::arch::asm!(
            "csrr {0}, mcause",
            "csrr {1}, mepc",
            out(reg) mcause,
            out(reg) mepc,
        );
    }
    if mcause != 0 {
        say(b"trap ");
        say_hex(mcause);
        say(b" at ");
        say_hex(mepc);
        put(b'\n');
        // Said once: the core's reset line leaves the CSRs as they
        // were (issue 419), so a later start would repeat a stale cause.
        unsafe {
            core::arch::asm!("csrw mcause, zero", "csrw mepc, zero");
        }
    }
    let mut again = false;
    loop {
        if again {
            say(b"boot again\n");
        } else {
            say(b"boot\n");
        }
        again = true;
        // The magic word, one byte at a time, so that a stream which
        // starts late or is typed by hand finds its footing rather
        // than failing once and giving up.
        let mut seen: u32 = 0;
        while seen != MAGIC {
            seen = (seen >> 8) | (get() << 24);
        }
        say(b"load\n");

        let addr = get_word();
        let len = get_word();
        if len == 0 || len & 3 != 0 || len > MAX_BYTES || !in_ram(addr, len) {
            say(b"bad len ");
            say_hex(addr);
            put(b' ');
            say_hex(len);
            put(b'\n');
            continue;
        }

        // The words, written as they arrive, and summed as they are
        // written so that nothing has to be read back. A block at a
        // time, with an acknowledgement between blocks, so the sender
        // never has to guess how fast this is.
        let mut sum: u32 = 0;
        let mut at = addr;
        let end = addr + len;
        let mut in_block = 0;
        put(ACK);
        while at < end {
            let word = get_word();
            unsafe { write_volatile(at as *mut u32, word) };
            sum = sum.wrapping_add(word);
            at += 4;
            in_block += 1;
            if in_block == BLOCK_WORDS {
                in_block = 0;
                put(ACK);
            }
        }
        if in_block != 0 {
            put(ACK);
        }

        let want = get_word();
        if want != sum {
            say(b"bad sum ");
            say_hex(sum);
            put(b' ');
            say_hex(want);
            put(b'\n');
            continue;
        }

        say(b"ok ");
        say_hex(addr);
        put(b'\n');
        // Wait for the last byte to leave the line before the jump, so
        // that a program which starts by saying something does not cut
        // this message in half.
        unsafe {
            while UART.add(UART_STATUS).read_volatile() & TX_BUSY != 0 {}
            let entry: extern "C" fn() -> ! = core::mem::transmute(addr as usize);
            entry()
        }
    }
}

/// The entry point, at address zero, which is where the core's program
/// counter starts.
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
