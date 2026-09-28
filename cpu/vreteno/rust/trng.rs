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
use vreteno_hal::{csr, entry, halt, map, Uart};
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
    let mut at = [0u32; RUN];
    capture(&mut win, &mut at);
    send_run(&win, &mut at);
    measure();
    halt()
}

/// Windows of `raw` read as fast as a loop goes, each with the cycle
/// it was asked for (issue 805). `raw` is the last 32 samples, one a
/// cycle, the newest in bit 0, so a window read `s` cycles after the
/// one before holds `s` new samples in its low bits and, above them,
/// the older `32 - s` where the one before had them.
fn capture(win: &mut [u32], at: &mut [u32]) {
    for (w, t) in win.iter_mut().zip(at.iter_mut()) {
        *t = csr::cycles();
        *w = unsafe { read_volatile(TRNG.add(RAW)) };
    }
}

/// The counts the cycle counter loses between two reads of it: one, on
/// the read itself (issue 807). When that is fixed, this is zero, and
/// the simulated check says so by finding every pair moved by one.
const CSR_LOSS: u32 = 1;

/// How far from the cycle count a pair's shift is looked for when the
/// count does not fit: the load behind the count can take a cycle or
/// two more or less than the one before, on the board's memory.
const SLACK: u32 = 3;

/// Whether window `b`, read `s` samples after `a`, holds `a`'s newer
/// samples as its older ones.
fn fits(a: u32, b: u32, s: u32) -> bool {
    match s {
        1..=31 => (b >> s) & (u32::MAX >> s) == a & (u32::MAX >> s),
        32 => true,
        _ => false,
    }
}

/// The samples between window `i - 1` and window `i`, or zero where the
/// run breaks. The cycle count says it first; the overlap confirms it,
/// or else picks the one shift within `SLACK` of it that fits, and if
/// none or more than one fits, which a source with a period can do,
/// the pair is not placed. A count over 32 skipped samples.
fn place(a: u32, b: u32, d: u32) -> u32 {
    if d > 32 {
        return 0;
    }
    if fits(a, b, d) {
        return d;
    }
    let lo = d.saturating_sub(SLACK).max(1);
    let hi = (d + SLACK).min(32);
    let mut found = 0;
    for s in lo..=hi {
        if fits(a, b, s) {
            if found != 0 {
                return 0;
            }
            found = s;
        }
    }
    found
}

/// Places every pair, then sends what was found and the longest run of
/// placed pairs. First a line of counts: `rawrun pairs P fit F moved
/// M gap G bad B`, where a pair fits at its cycle count, is moved to
/// a nearby shift by its overlap, has a gap of more than 32 cycles, or
/// is bad, fitting no shift or more than one near its count. Then, for
/// the first 16 that did not fit, `pair I d D s S`, the count and the
/// shift, zero if none. Then the run under `rawrun`, 32 samples to a
/// line, the oldest in bit 31: its first window whole, then the new
/// samples of each after it, what is left short of a line dropped.
/// `at` holds the cycle counts and is overwritten with the shifts.
fn send_run(win: &[u32], at: &mut [u32]) {
    let (mut fit, mut moved, mut gap, mut bad) = (0u32, 0u32, 0u32, 0u32);
    let mut odd = 0;
    for i in (1..win.len()).rev() {
        at[i] = at[i].wrapping_sub(at[i - 1]).wrapping_add(CSR_LOSS);
    }
    for i in 1..win.len() {
        let d = at[i];
        let s = place(win[i - 1], win[i], d);
        match (s == d, s, d > 32) {
            (true, _, _) => fit += 1,
            (false, 0, true) => gap += 1,
            (false, 0, false) => bad += 1,
            _ => moved += 1,
        }
        if s != d && odd < 16 {
            odd += 1;
            Uart::say(b"pair ");
            Uart::put_decimal(i as u32);
            Uart::say(b" d ");
            Uart::put_decimal(d);
            Uart::say(b" s ");
            Uart::put_decimal(s);
            Uart::put(b'\n');
        }
        at[i] = s;
    }
    Uart::say(b"rawrun pairs ");
    Uart::put_decimal(win.len() as u32 - 1);
    for (name, n) in [
        (&b" fit "[..], fit),
        (b" moved ", moved),
        (b" gap ", gap),
        (b" bad ", bad),
    ] {
        Uart::say(name);
        Uart::put_decimal(n);
    }
    Uart::put(b'\n');
    // The longest stretch of windows joined by placed pairs.
    let (mut best, mut best_len, mut start) = (0, 1, 0);
    for i in 1..=win.len() {
        if i == win.len() || at[i] == 0 {
            if i - start > best_len {
                best = start;
                best_len = i - start;
            }
            start = i;
        }
    }
    if best_len < 2 {
        return;
    }
    Uart::say(b"rawrun\n");
    Uart::put_hex(win[best]);
    Uart::put(b'\n');
    let (mut acc, mut n) = (0u64, 0u32);
    for i in best + 1..best + best_len {
        let s = at[i];
        acc = (acc << s) | u64::from(win[i] & (u32::MAX >> (32 - s)));
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
