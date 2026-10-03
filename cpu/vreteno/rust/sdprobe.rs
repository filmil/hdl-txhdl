// SPDX-License-Identifier: Apache-2.0
//! The card in the SD slot, brought up by the core and read (issue
//! 153), printed on the serial port, and nothing written.
//!
//! * `cmd8` and what the card echoed of `0x1aa`: a card of version 2
//!   or later echoes it whole, and only such a card can be high
//!   capacity. `no card` if nothing answers `CMD0` and `CMD8`.
//! * `ocr` and the operating conditions the card answered `ACMD41`
//!   with, once it said it was ready, then `sdhc` or `sdsc` from the
//!   capacity bit. The program asks with HCS set, as a host that can
//!   address a card above 2 GB by block must.
//! * `cid` and the card's identity, four words, the manufacturer
//!   first; `rca` and the address the card took for itself.
//! * `blk0`, then block 0 read on one data line, sixteen bytes to a
//!   line, then `wide same` or `wide differs` for the same block read
//!   again on four lines, which is what shows all four lines carry
//!   what the card sends.
//! * `boot 55aa` if the block ends with the boot signature a
//!   partition table carries, `boot none` if not.
//!
//! The card is identified at 400 kHz, as the specification asks of a
//! card not yet known, and read at 12.5 MHz. At 25 MHz a read has 20 ns
//! from the card clock's fall, which the host makes, to the host's
//! sample: the clock's way out, the card's 14 ns to drive at default
//! speed, and the data's way back, about as long as the window. At
//! 12.5 MHz the window is 40 ns. Block 0 is address 0 on a
//! card of either capacity, so the read needs no case for either.
//!
//! The program only reads. It sends no write command, so the card
//! holds afterwards exactly what it held before.
#![no_std]
#![no_main]

use core::ptr::{read_volatile, write_volatile};
use vreteno_hal::{entry, halt, map, Uart};
use vreteno_regs::sd;

const SD: *mut u32 = map::SD as *mut u32;

/// A half of a card clock is `DIV + 1` cycles of the board's 100 MHz:
/// 400 kHz to identify the card, 12.5 MHz to read it.
const SLOW: u32 = 124;
const FAST: u32 = 3;

/// The command word's response field: none, 48 bits, 136 bits.
const SHORT: u32 = 1 << sd::CMD_RESP_SHIFT;
const LONG: u32 = 2 << sd::CMD_RESP_SHIFT;

/// What a status word says went wrong.
const FAULTS: u32 = sd::STATUS_RTIMEOUT_MASK
    | sd::STATUS_RCRC_MASK
    | sd::STATUS_DTIMEOUT_MASK
    | sd::STATUS_DCRC_MASK;

/// How many times `ACMD41` is asked before the card is given up on:
/// a thousand and more milliseconds at 400 kHz, which is what the
/// specification allows a card to take.
const TRIES: u32 = 2000;

/// The words of a block.
const WORDS: usize = 128;

entry!(main);

fn reg(at: usize) -> u32 {
    unsafe { read_volatile(SD.add(at / 4)) }
}

fn set(at: usize, v: u32) {
    unsafe { write_volatile(SD.add(at / 4), v) }
}

/// One command, waited for. Answers its status with the done bit
/// cleared afterwards, as the host leaves it.
fn command(index: u32, arg: u32, flags: u32) -> u32 {
    set(sd::ARG, arg);
    set(sd::CMD, index | flags);
    loop {
        let s = reg(sd::STATUS);
        if s & sd::STATUS_DONE_MASK != 0 {
            set(sd::STATUS, sd::STATUS_DONE_MASK);
            return s;
        }
    }
}

/// A command that must go through, or the program stops saying which.
fn must(index: u32, arg: u32, flags: u32) {
    let s = command(index, arg, flags);
    if s & FAULTS != 0 {
        Uart::say(b"cmd");
        Uart::put_decimal(index);
        Uart::say(b" failed ");
        Uart::put_hex(s);
        Uart::put(b'\n');
        halt()
    }
}

/// Block 0 into `words`, as the card sends it, on whichever lines the
/// host is set to.
fn block0(words: &mut [u32; WORDS]) {
    must(17, 0, SHORT | sd::CMD_READ_MASK);
    for w in words.iter_mut() {
        *w = reg(sd::DATA);
    }
}

/// Two hexadecimal digits.
fn put_byte(v: u32) {
    for shift in [4, 0] {
        let d = ((v >> shift) & 0xf) as u8;
        Uart::put(if d < 10 { b'0' + d } else { b'a' + d - 10 });
    }
}

fn main() -> ! {
    set(sd::CTRL, SLOW);
    // The clocks a card wants after power before it listens, then
    // reset to the idle state.
    command(0, 0, sd::CMD_CLOCKS_MASK);
    command(0, 0, 0);
    let s = command(8, 0x1aa, SHORT);
    if s & FAULTS != 0 {
        Uart::say(b"no card\n");
        halt()
    }
    Uart::say(b"cmd8 ");
    Uart::put_hex(reg(sd::RESP0) & 0xfff);
    Uart::put(b'\n');
    // Ready, asked with HCS set; the answer's CRC field is all ones.
    let mut ocr = 0;
    for _ in 0..TRIES {
        must(55, 0, SHORT);
        must(41, 0x4030_0000, SHORT | sd::CMD_NOCRC_MASK);
        ocr = reg(sd::RESP0);
        if ocr & 0x8000_0000 != 0 {
            break;
        }
    }
    if ocr & 0x8000_0000 == 0 {
        Uart::say(b"never ready\n");
        halt()
    }
    Uart::say(b"ocr ");
    Uart::put_hex(ocr);
    Uart::say(if ocr & 0x4000_0000 != 0 {
        b" sdhc\n"
    } else {
        b" sdsc\n"
    });
    must(2, 0, LONG);
    Uart::say(b"cid");
    for at in [sd::RESP3, sd::RESP2, sd::RESP1, sd::RESP0] {
        Uart::put(b' ');
        Uart::put_hex(reg(at));
    }
    Uart::put(b'\n');
    must(3, 0, SHORT);
    let rca = reg(sd::RESP0) >> 16;
    Uart::say(b"rca ");
    Uart::put_hex(rca);
    Uart::put(b'\n');
    must(7, rca << 16, SHORT | sd::CMD_BUSY_MASK);
    must(16, 512, SHORT);
    set(sd::CTRL, FAST);
    let mut one = [0u32; WORDS];
    block0(&mut one);
    Uart::say(b"blk0\n");
    for line in one.chunks(4) {
        for w in line {
            for shift in [24, 16, 8, 0] {
                put_byte(w >> shift);
            }
        }
        Uart::put(b'\n');
    }
    // Four lines: the card told first, then the host.
    must(55, rca << 16, SHORT);
    must(6, 2, SHORT);
    set(sd::CTRL, FAST | sd::CTRL_WIDE_MASK);
    let mut four = [0u32; WORDS];
    block0(&mut four);
    Uart::say(if four == one {
        b"wide same\n"
    } else {
        b"wide differs\n"
    });
    Uart::say(if one[WORDS - 1] & 0xffff == 0x55aa {
        b"boot 55aa\n"
    } else {
        b"boot none\n"
    });
    halt()
}
