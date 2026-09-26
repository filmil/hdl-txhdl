// SPDX-License-Identifier: Apache-2.0
//! A test of the board's entropy source, run by the core: the source
//! turned on, four words taken, and a line on the serial port that says
//! whether they came and differed. The core halts either way, after
//! the line.
//!
//! Built with `--cfg=board_run`, for the board, the program then
//! measures the source: it reads the raw samples, the folded bits
//! before the extractor, 4096 words of them, and sends each up the
//! serial line as eight hex digits, followed by 4096 words of the
//! extractor's output the same way. That is what a host estimates the
//! source's bias and entropy from, and the only evidence there is that
//! the rings are random rather than merely running (issue 458). A
//! simulation runs the model rings and says nothing here, since what
//! it would measure is the model.
#![no_std]
#![no_main]

use core::ptr::{read_volatile, write_volatile};
use vreteno_hal::{entry, halt, map, Uart};
use vreteno_regs::trng;

/// The source's four words, as word indices from its base, and its
/// bits, all as its map declares them (issue 709).
const TRNG: *mut u32 = map::TRNG as *mut u32;
const DATA: usize = trng::DATA / 4;
const STATUS: usize = trng::STATUS / 4;
const CTRL: usize = trng::CTRL / 4;
#[cfg(board_run)]
const RAW: usize = trng::RAW / 4;

const STATUS_READY: u32 = trng::STATUS_READY_MASK;
const STATUS_FAULT: u32 = trng::STATUS_FAULT_MASK;
const CTRL_RUN: u32 = trng::CTRL_RUN_MASK;

entry!(main);

fn status() -> u32 {
    unsafe { read_volatile(TRNG.add(STATUS)) }
}

/// A word of entropy, waited for. `None` if the health test tripped.
fn word() -> Option<u32> {
    loop {
        let s = status();
        if s & STATUS_FAULT != 0 {
            return None;
        }
        if s & STATUS_READY != 0 {
            return Some(unsafe { read_volatile(TRNG.add(DATA)) });
        }
    }
}

fn main() -> ! {
    unsafe { write_volatile(TRNG.add(CTRL), CTRL_RUN) };
    Uart::say(b"trng ");
    let mut words = [0u32; 4];
    for w in words.iter_mut() {
        match word() {
            Some(v) => *w = v,
            None => {
                Uart::say(b"fault\n");
                halt()
            }
        }
    }
    // Four words, every pair different.
    let mut same = false;
    for (i, a) in words.iter().enumerate() {
        for b in words.iter().take(i) {
            if a == b {
                same = true;
            }
        }
    }
    Uart::say(if same { b"bad\n" } else { b"ok\n" });
    measure();
    halt()
}

/// The measurement, for the board: the raw samples and the words.
#[cfg(board_run)]
fn measure() {
    const WORDS: usize = 4096;
    Uart::say(b"raw\n");
    for _ in 0..WORDS {
        // A byte on the line is 87 microseconds, so the eight of the
        // last word spaced the reads well past the 32 cycles a raw
        // word takes to fill: every word read is a fresh one.
        let raw = unsafe { read_volatile(TRNG.add(RAW)) };
        Uart::put_hex(raw);
        Uart::put(b'\n');
    }
    Uart::say(b"words\n");
    for _ in 0..WORDS {
        match word() {
            Some(v) => {
                Uart::put_hex(v);
                Uart::put(b'\n');
            }
            None => {
                Uart::say(b"fault\n");
                return;
            }
        }
    }
    Uart::say(b"end\n");
}

#[cfg(not(board_run))]
fn measure() {}
