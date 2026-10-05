// SPDX-License-Identifier: Apache-2.0
//! The video scanout on the board, under load (issue 151).
//!
//! The program paints a 640 by 480 frame into the DDR3, colour bars
//! over a grey ramp with the TxHDL logo in the bottom right corner, a
//! logo pixel to a screen pixel on a black square of its own, points
//! the scanout at it and shows it. Then it reads the scanout's
//! underflow bit twice: once after two seconds on an otherwise idle
//! bus, and once after ten seconds in which the core copies a quarter
//! of a megabyte back and forth in the DDR3 without a pause and the
//! Ethernet port sends a full frame from the DDR3 whenever it is
//! ready for one. The bit is sticky, so a single line that came late
//! in those ten seconds shows.
//!
//! What it prints, a line each: `scan idle` and the bit, `scan load`
//! and the bit with the copies and frames done, and at the end
//! `scan ok`, `scan underflow`, or `scan stuck` with the address of a
//! line that never came (issue 1197). The picture stays on the screen.
#![no_std]
#![no_main]

use core::ptr::{read_volatile, write_volatile};
use vreteno_hal::{entry, halt, Scan, Timer, Uart};

entry!(main);

/// The frame, well above the program and the Ethernet port's slots.
const FRAME: u32 = 0x4200_0000;
/// Words from one row of the frame to the next, of which the first
/// `Scan::WIDTH` are shown: the scanout's stride, which is Razboj's row
/// (issue 985).
const ROW: u32 = Scan::STRIDE / 4;
/// The copy's two halves.
const FROM: u32 = 0x4300_0000;
const TO: u32 = 0x4304_0000;
const COPY_WORDS: u32 = 0x1_0000;
/// One second of the timer.
const SECOND: u32 = 100_000_000;

/// The Ethernet port's registers and its first transmit slot, as
/// `ethtx.rs` has them.
const ETH: *mut u32 = 0x3400 as *mut u32;
const TX_SLOT: usize = vreteno_regs::ethslots::TX_SLOT / 4;
const TX_LENGTH: usize = vreteno_regs::ethslots::TX_LENGTH / 4;
const TX_START: usize = vreteno_regs::ethslots::TX_START / 4;
const TX_READY: usize = vreteno_regs::ethslots::TX_READY / 4;
const TXBUF: u32 = 0x4100_1000;
/// A full frame: to every station, from nobody, with an EtherType no
/// stack takes, so the machine at the other end counts it and drops it.
const FRAME_LEN: u32 = 1514;

/// The logo's place: eight in from the bottom right corner.
const LOGO_LEFT: u32 = Scan::WIDTH - txhdl_logo::W as u32 - 8;
const LOGO_TOP: u32 = Scan::HEIGHT - txhdl_logo::H as u32 - 8;

/// The picture at a pixel, before the logo: eight bars over the top
/// two thirds, a grey ramp under them, and a black square in the
/// bottom right corner for the logo. Its light grey wordmark was faint
/// over the ramp's light end (issue 1245); on black it reads, as it
/// does on the icosahedron's.
fn picture(x: u32, y: u32) -> u32 {
    if x + 8 >= LOGO_LEFT && y + 8 >= LOGO_TOP {
        0
    } else if y < 320 {
        let bars = [
            0xffffff, 0xffff00, 0x00ffff, 0x00ff00, 0xff00ff, 0xff0000,
            0x0000ff, 0x000000,
        ];
        let at = (x >> 4) * 8 / 40;
        let mut i = 0usize;
        let mut c = bars[0];
        while i < 8 {
            if i as u32 == at {
                c = bars[i];
            }
            i += 1;
        }
        c
    } else {
        let g = x * 256 / 640;
        g << 16 | g << 8 | g
    }
}

/// The frame painted, the logo over it, and the last word read back so
/// that the posted stores have landed before the scanout reads them.
fn paint() {
    let base = FRAME as *mut u32;
    let mut y = 0u32;
    while y < Scan::HEIGHT {
        let mut x = 0u32;
        while x < Scan::WIDTH {
            let at = (y * ROW + x) as usize;
            unsafe { write_volatile(base.add(at), picture(x, y)) };
            x += 1;
        }
        y += 1;
    }
    // The logo, a pixel of its own to a pixel of the screen, on its
    // square (issue 1213).
    let mut r = 0usize;
    while r < txhdl_logo::H {
        let mut c = 0usize;
        while c < txhdl_logo::W {
            if let Some(px) = txhdl_logo::colour(c, r) {
                let at = ((LOGO_TOP + r as u32) * ROW + LOGO_LEFT + c as u32)
                    as usize;
                unsafe { write_volatile(base.add(at), px) };
            }
            c += 1;
        }
        r += 1;
    }
    let last = ((Scan::HEIGHT - 1) * ROW + Scan::WIDTH - 1) as usize;
    let _ = unsafe { read_volatile(base.add(last)) };
}

/// The frame the port sends: a broadcast header and every other byte
/// its index, written once into the first transmit slot.
fn frame() {
    let slot = TXBUF as *mut u32;
    let words = (FRAME_LEN + 3) >> 2;
    let mut k = 0u32;
    while k < words {
        let mut w = 0u32;
        let mut lane = 0u32;
        while lane < 4 {
            let i = k * 4 + lane;
            let b = if i < 6 {
                0xff
            } else if i == 12 {
                0x88
            } else if i == 13 {
                0xb5
            } else {
                i & 0xff
            };
            w |= b << (lane * 8);
            lane += 1;
        }
        unsafe { write_volatile(slot.add(k as usize), w) };
        k += 1;
    }
    let _ = unsafe { read_volatile(slot.add((words - 1) as usize)) };
}

/// Send the frame if the port is ready for it; say whether it was.
fn send() -> bool {
    let ready = unsafe { read_volatile(ETH.add(TX_READY)) } & 1 == 1;
    if ready {
        unsafe {
            write_volatile(ETH.add(TX_SLOT), 0);
            write_volatile(ETH.add(TX_LENGTH), FRAME_LEN);
            write_volatile(ETH.add(TX_START), 1);
        }
    }
    ready
}

/// One pass of the copy, one way, offering the port a frame every 256
/// words, so that it sends whenever it is ready rather than once a
/// pass. Returns the frames it sent.
fn copy(from: u32, to: u32) -> u32 {
    let src = from as *const u32;
    let dst = to as *mut u32;
    let mut sent = 0u32;
    let mut k = 0usize;
    while k < COPY_WORDS as usize {
        unsafe { write_volatile(dst.add(k), read_volatile(src.add(k))) };
        if k & 0xff == 0 && send() {
            sent += 1;
        }
        k += 1;
    }
    sent
}

fn say_bit(label: &[u8], on: bool) {
    Uart::say(label);
    Uart::say(if on { b" 1" } else { b" 0" });
}

fn main() -> ! {
    Uart::say(b"scan probe\n");
    paint();
    frame();
    Scan::base(FRAME);
    Scan::show(true);
    // Two frames for the base to be taken and the first lines asked
    // for, then a clean start.
    Timer::wait(SECOND / 10);
    Scan::clear();
    Timer::wait(2 * SECOND);
    let idle = Scan::underflow();
    say_bit(b"scan idle", idle);
    Uart::put(b'\n');

    Scan::clear();
    let start = Timer::ticks();
    let mut copies = 0u32;
    let mut frames = 0u32;
    while Timer::ticks().wrapping_sub(start) < 10 * SECOND {
        frames += if copies & 1 == 0 {
            copy(FROM, TO)
        } else {
            copy(TO, FROM)
        };
        copies += 1;
    }
    let load = Scan::underflow();
    say_bit(b"scan load", load);
    Uart::say(b" copies ");
    Uart::put_decimal(copies);
    Uart::say(b" frames ");
    Uart::put_decimal(frames);
    Uart::put(b'\n');
    // A line that never came, which a clear underflow bit cannot say
    // (issue 1197).
    let stuck = Scan::stuck();
    if let Some(at) = stuck {
        Uart::say(b"scan stuck ");
        Uart::put_hex(at);
        Uart::put(b'\n');
    } else if idle || load {
        Uart::say(b"scan underflow\n");
    } else {
        Uart::say(b"scan ok\n");
    }
    halt()
}
