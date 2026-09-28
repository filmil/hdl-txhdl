// SPDX-License-Identifier: Apache-2.0
//! A test of the board's entropy source, run by the core: the source
//! turned on, four words taken, and a line on the serial port that says
//! whether they came and differed. The core halts either way, after
//! the line.
//!
//! Then it reads the raw samples in a tight loop, windows of 32 that
//! overlap, and sends them joined as one run of samples in a row, under
//! `rawrun`, so a host can measure lags longer than a word (issue
//! 805). In simulation that run is the model's, which the board test
//! checks sample for sample.
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
const RAW: usize = trng::RAW / 4;

const STATUS_READY: u32 = trng::STATUS_READY_MASK;
const STATUS_FAULT: u32 = trng::STATUS_FAULT_MASK;
const CTRL_RUN: u32 = trng::CTRL_RUN_MASK;

/// The windows the run is read from: on the board, enough for a few
/// thousand words of samples in a row; in simulation, what the boot
/// memory's stack holds with room.
#[cfg(board_run)]
const RUN: usize = 4096;
#[cfg(not(board_run))]
const RUN: usize = 64;

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
    // The run of samples in a row: the whole capture first, then the
    // sending, since a byte on the line is thousands of cycles.
    let mut win = [0u32; RUN];
    capture(&mut win);
    send_run(&win);
    measure();
    halt()
}

/// Windows of `raw` read as fast as a loop goes (issue 805). `raw` is
/// the last 32 samples, one a cycle, the newest in bit 0, so a window
/// read `s` cycles after the one before holds `s` new samples in its
/// low bits and, above them, the older `32 - s` where the one before
/// had them.
fn capture(win: &mut [u32]) {
    for w in win.iter_mut() {
        *w = unsafe { read_volatile(TRNG.add(RAW)) };
    }
}

/// The fewest samples of overlap two windows are matched on: a shift
/// wider than 24 leaves fewer than 8, which could agree by chance.
const MAX_SHIFT: u32 = 24;

/// The samples between one window and the next, found from the
/// windows themselves: the smallest shift at which every window's
/// older bits are the one before's newer ones. The loop is the same
/// every time round, so the shift is one number for all of them.
/// `None` if no shift up to `MAX_SHIFT` fits every pair, which would
/// say the loop is too slow and samples were skipped, or if the
/// windows are all alike, which fits every shift and places nothing.
/// The cycle counter would answer this too, but it loses a count on
/// every read of it (issue 807), so the samples are the clock.
fn shift(win: &[u32]) -> Option<u32> {
    if win.iter().all(|w| *w == win[0]) {
        return None;
    }
    (1..=MAX_SHIFT).find(|&s| {
        let keep = u32::MAX >> s;
        win.windows(2).all(|p| (p[1] >> s) & keep == p[0] & keep)
    })
}

/// Sends the windows as one run of samples in a row, 32 to a line, the
/// oldest in bit 31, under `rawrun`: the first window whole, then the
/// `s` new samples of each after it. What is left short of a line at
/// the end is dropped. A line before it says the shift, or
/// `rawrun bad` says no shift fitted and nothing follows.
fn send_run(win: &[u32]) {
    let Some(s) = shift(win) else {
        Uart::say(b"rawrun bad\n");
        return;
    };
    Uart::say(b"shift ");
    Uart::put_decimal(s);
    Uart::say(b"\nrawrun\n");
    Uart::put_hex(win[0]);
    Uart::put(b'\n');
    let (mut acc, mut n) = (0u64, 0u32);
    for w in &win[1..] {
        acc = (acc << s) | u64::from(w & (u32::MAX >> (32 - s)));
        n += s;
        if n >= 32 {
            n -= 32;
            Uart::put_hex((acc >> n) as u32);
            Uart::put(b'\n');
        }
    }
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
