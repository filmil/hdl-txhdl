// SPDX-License-Identifier: Apache-2.0
//! The Ethernet port's throughput on the board, through the slots and
//! the DMA engines, against the core's own copy of each frame
//! (issue 1038, the Ethernet half of #151).
//!
//! The board has no register path to the MAC: `EthLite` is not on it.
//! So the baseline is the copy any register path would have to make at
//! least, a frame's words written into a slot or read out of one by the
//! core, timed on its own. A path that moved a byte per bus transaction
//! would cost more than that.
//!
//! Send, for frames of 60 and of 1514 bytes, a thousand each:
//!
//! * `tx dma`: the frame is written into both transmit slots once, and
//!   the core only hands the engine a slot and a length per frame;
//! * `tx copy`: the core writes each frame into its slot before
//!   handing it over, as a driver copying a packet does.
//!
//! Receive, four bursts of a thousand numbered frames from the machine
//! at the other end of the cable, `ethtest send` in `//eth/hil`, in the
//! order 60, 60, 1514, 1514: the first of each size is only noted, the
//! second is copied word by word out of its slot into memory.
//!
//! Each line gives the frames, the timer's ticks from the first to the
//! last (a hundred million a second), the frames and kilobytes a
//! second, and the core's cycles a frame spent on the frame itself, not
//! waiting for the port: the register writes, the copy, the reads. A
//! receive line also says how many of the thousand arrived and how many
//! the port lost.
#![no_std]
#![no_main]

use core::ptr::{read_volatile, write_volatile};
use vreteno_hal::{csr, entry, halt, Timer, Uart};

entry!(main);

const SECOND: u32 = 100_000_000;
const FRAMES: u32 = 1000;

/// The port's registers, as `ethtx.rs` and `ethrx.rs` have them.
const ETH: *mut u32 = 0x3400 as *mut u32;
const RX_SLOT: usize = vreteno_regs::ethslots::RX_SLOT / 4;
const RX_LENGTH: usize = vreteno_regs::ethslots::RX_LENGTH / 4;
const RX_EV_PENDING: usize = vreteno_regs::ethslots::RX_EV_PENDING / 4;
const TX_SLOT: usize = vreteno_regs::ethslots::TX_SLOT / 4;
const TX_LENGTH: usize = vreteno_regs::ethslots::TX_LENGTH / 4;
const TX_START: usize = vreteno_regs::ethslots::TX_START / 4;
const TX_READY: usize = vreteno_regs::ethslots::TX_READY / 4;
/// The slots in the DDR3, two a direction, 2 KiB each.
const RXBUF: u32 = 0x4100_0000;
const TXBUF: u32 = 0x4100_1000;
const SLOT_BYTES: u32 = 0x800;
/// Where a received frame is copied to, above the program.
const COPY_TO: u32 = 0x4300_0000;

/// The frames' EtherType, which no stack takes and which is not
/// `0x88b5`: the port hands that one to the remote peripheral, not to
/// the slots.
const PERF_TYPE: u32 = 0x88b6;
/// The magic `ethtest` puts first in a frame's payload, and the byte
/// its sequence number starts at, big-endian, after the header.
const MAGIC: &[u8] = b"TXHDL-PERF";
const SEQ_AT: u32 = 14 + 10;

fn reg(r: usize) -> u32 {
    unsafe { read_volatile(ETH.add(r)) }
}

fn set(r: usize, v: u32) {
    unsafe { write_volatile(ETH.add(r), v) }
}

/// A frame's byte at `i`: to every station, from a locally
/// administered address, this test's type, the magic, the number and a
/// pattern.
fn byte(i: u32, seq: u32) -> u32 {
    let source = [0x02u32, 0x00, 0x54, 0x58, 0x48, 0x4c];
    match i {
        0..=5 => 0xff,
        6..=11 => source[(i - 6) as usize],
        12 => PERF_TYPE >> 8,
        13 => PERF_TYPE & 0xff,
        14..=23 => MAGIC[(i - 14) as usize] as u32,
        24 => (seq >> 8) & 0xff,
        25 => seq & 0xff,
        _ => (seq * 31 + i) & 0xff,
    }
}

/// The frame numbered `seq`, `len` bytes, into transmit slot `slot`,
/// read back at its last word since stores are posted.
fn fill(slot: u32, len: u32, seq: u32) {
    let at = (TXBUF + slot * SLOT_BYTES) as *mut u32;
    let words = (len + 3) / 4;
    let mut k = 0u32;
    while k < words {
        let mut w = 0u32;
        let mut lane = 0u32;
        while lane < 4 {
            w |= byte(k * 4 + lane, seq) << (lane * 8);
            lane += 1;
        }
        unsafe { write_volatile(at.add(k as usize), w) };
        k += 1;
    }
    let _ = unsafe { read_volatile(at.add((words - 1) as usize)) };
}

/// The words of a frame, made once, which the copy writes from: a
/// driver copies from a packet that is already made.
static mut FRAME: [u32; 379] = [0; 379];

fn make(len: u32) {
    let words = (len + 3) / 4;
    let mut k = 0u32;
    while k < words {
        let mut w = 0u32;
        let mut lane = 0u32;
        while lane < 4 {
            w |= byte(k * 4 + lane, 0) << (lane * 8);
            lane += 1;
        }
        unsafe { (*core::ptr::addr_of_mut!(FRAME))[k as usize] = w };
        k += 1;
    }
}

/// The made frame into a transmit slot, word by word.
fn copy_in(slot: u32, len: u32) {
    let at = (TXBUF + slot * SLOT_BYTES) as *mut u32;
    let words = (len + 3) / 4;
    let from = core::ptr::addr_of!(FRAME) as *const u32;
    let mut k = 0usize;
    while k < words as usize {
        unsafe { write_volatile(at.add(k), read_volatile(from.add(k))) };
        k += 1;
    }
    let _ = unsafe { read_volatile(at.add(words as usize - 1)) };
}

/// A received frame out of its slot into memory, word by word.
fn copy_out(slot: u32, len: u32) {
    let from = (RXBUF + slot * SLOT_BYTES) as *const u32;
    let to = COPY_TO as *mut u32;
    let words = (len + 3) / 4;
    let mut k = 0usize;
    while k < words as usize {
        unsafe { write_volatile(to.add(k), read_volatile(from.add(k))) };
        k += 1;
    }
}

/// `n` a second over `ticks`, and the same in kilobytes for frames of
/// `len` bytes.
fn say_rates(n: u32, len: u32, ticks: u32) {
    let t = ticks.max(1) as u64;
    let per_s = n as u64 * SECOND as u64 / t;
    let kb_s = n as u64 * len as u64 * SECOND as u64 / t / 1000;
    Uart::say(b" frames/s ");
    Uart::put_decimal(per_s as u32);
    Uart::say(b" kB/s ");
    Uart::put_decimal(kb_s as u32);
}

/// Send a thousand frames of `len` bytes, the core copying each into
/// its slot first or not.
fn send(len: u32, copy: bool) {
    make(len);
    fill(0, len, 0);
    fill(1, len, 1);
    let mut work = 0u32;
    let start = Timer::ticks();
    let mut i = 0u32;
    while i < FRAMES {
        let slot = i & 1;
        if copy {
            // The engine may still be sending the other slot's frame,
            // so the copy overlaps it, as a driver with two buffers
            // does; this slot's last frame went before the port was
            // last ready.
            let c = csr::cycles();
            copy_in(slot, len);
            work = work.wrapping_add(csr::cycles().wrapping_sub(c));
        }
        while reg(TX_READY) & 1 == 0 {}
        let c = csr::cycles();
        set(TX_SLOT, slot);
        set(TX_LENGTH, len);
        set(TX_START, 1);
        work = work.wrapping_add(csr::cycles().wrapping_sub(c));
        i += 1;
    }
    // The last frame gone. The fence waits until the start's write
    // has been answered, so the port says busy from then until the
    // frame is out, or ready if it already is.
    unsafe { core::arch::asm!("fence") };
    while reg(TX_READY) & 1 == 0 {}
    let ticks = Timer::ticks().wrapping_sub(start);
    Uart::say(if copy { b"tx copy " } else { b"tx dma " });
    Uart::put_decimal(len);
    Uart::say(b" frames ");
    Uart::put_decimal(FRAMES);
    Uart::say(b" ticks ");
    Uart::put_decimal(ticks);
    say_rates(FRAMES, len, ticks);
    Uart::say(b" cycles/frame ");
    Uart::put_decimal(work / FRAMES);
    Uart::put(b'\n');
}

/// Receive one burst of numbered frames of `len` bytes: from the first
/// frame, until the thousandth or half a second with none. Waits up to
/// forty seconds for the first, which is as long as the timer's low
/// word can count.
fn receive(len: u32, copy: bool) {
    Uart::say(b"rx waiting ");
    Uart::put_decimal(len);
    Uart::put(b'\n');
    let mut seen = [0u32; (FRAMES as usize + 31) / 32];
    let (mut got, mut fresh, mut other) = (0u32, 0u32, 0u32);
    let mut work = 0u32;
    let wait = Timer::ticks();
    let (mut first, mut last) = (0u32, 0u32);
    loop {
        let quiet = if got == 0 {
            Timer::ticks().wrapping_sub(wait) > 40 * SECOND
        } else {
            Timer::ticks().wrapping_sub(last) > SECOND / 2
        };
        if quiet || fresh == FRAMES {
            break;
        }
        if reg(RX_EV_PENDING) & 1 == 0 {
            continue;
        }
        let now = Timer::ticks();
        let c = csr::cycles();
        let n = reg(RX_LENGTH) & 0xffff;
        let slot = reg(RX_SLOT) & 1;
        let at = RXBUF + slot * SLOT_BYTES;
        // The type and the number, in two words: the frame's bytes 12
        // and 13, and 24 and 25.
        let w3 = unsafe { read_volatile((at + 12) as *const u32) };
        let w6 = unsafe { read_volatile((at + SEQ_AT) as *const u32) };
        let ty = (w3 & 0xff) << 8 | (w3 >> 8) & 0xff;
        let seq = (w6 & 0xff) << 8 | (w6 >> 8) & 0xff;
        if copy {
            copy_out(slot, n);
        }
        set(RX_EV_PENDING, 1);
        work = work.wrapping_add(csr::cycles().wrapping_sub(c));
        if ty != PERF_TYPE || n < len || seq >= FRAMES {
            other += 1;
            continue;
        }
        if got == 0 {
            first = now;
        }
        last = now;
        got += 1;
        let (i, b) = ((seq / 32) as usize, 1u32 << (seq % 32));
        if seen[i] & b == 0 {
            seen[i] |= b;
            fresh += 1;
        }
    }
    Uart::say(if copy { b"rx copy " } else { b"rx dma " });
    Uart::put_decimal(len);
    Uart::say(b" got ");
    Uart::put_decimal(fresh);
    Uart::say(b" of ");
    Uart::put_decimal(FRAMES);
    Uart::say(b" lost ");
    Uart::put_decimal(FRAMES - fresh);
    Uart::say(b" other ");
    Uart::put_decimal(other);
    let ticks = last.wrapping_sub(first);
    Uart::say(b" ticks ");
    Uart::put_decimal(ticks);
    if fresh > 1 {
        say_rates(fresh - 1, len, ticks);
    }
    Uart::say(b" cycles/frame ");
    Uart::put_decimal(work / (got + other).max(1));
    Uart::put(b'\n');
}

fn main() -> ! {
    Uart::say(b"ethperf\n");
    send(60, false);
    send(60, true);
    send(1514, false);
    send(1514, true);
    receive(60, false);
    receive(60, true);
    receive(1514, false);
    receive(1514, true);
    Uart::say(b"ethperf done\n");
    halt()
}
