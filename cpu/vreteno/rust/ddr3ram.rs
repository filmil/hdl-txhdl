// SPDX-License-Identifier: Apache-2.0
//! A test of the board's DDR3 memory that the loader sends (issue
//! 1174): the writes, the reads and the strobes of the path into the
//! memory, run on the flagship with no second bitstream.
//!
//! `ddr3.rs` is a boot-memory image that writes across the whole
//! gigabyte, `0x4000_0000` included, where the loader puts its
//! programs, so it runs only on the bitstream built around it. This one
//! leaves alone what a flagship session keeps in the memory: the loaded
//! program, from `0x4000_0000`, the Ethernet slots at `0x4100_0000`,
//! the device tree at `0x4100_8000` and Razboj's frame buffer at
//! `0x4200_0000`. It starts at `0x4100_2000`, and its high words are
//! above `0x4400_0000`.
//!
//! Four checks, a line each, and then the verdict:
//!
//! * `words`: a block of words written whole and then read back, each
//!   word its own address mixed with a constant, so that a write which
//!   lands on another word's address, or a read from the wrong one,
//!   shows;
//! * `high`: words spread up to the top of the gigabyte, each at a
//!   different offset into its stretch, so that a mistake in the high
//!   address bits shows as well as one in the low;
//! * `strobes`: bytes and halfwords written into known words, then the
//!   words read back whole, so that a strobe which writes too much or
//!   too little shows;
//! * `lanes`: those words read back again a byte and a halfword at a
//!   time, at every offset.
//!
//! A check's line is `ddr3ram <check> ok`, or `ddr3ram <check> bad`
//! with how many accesses were wrong, the first wrong address, and what
//! was read there. The last line is `ddr3ram ok` or `ddr3ram bad`. The
//! core halts when every check passed and spins when one did not, so
//! the board's first LED, which shows the halt, says the same.
//!
//! As in `ddr3.rs`, nothing here indexes a slice and nothing divides,
//! so nothing asks for `core`'s panic path.
#![no_std]
#![no_main]

use core::ptr::{read_volatile, write_volatile};
use vreteno_hal::{entry, halt, map, Uart};

entry!(main);

/// The block of words: from `0x4100_2000`, past the Ethernet slots.
const BLOCK: usize = map::DDR3 + 0x0100_2000;
/// Words in the block.
const BLOCK_WORDS: usize = 256;
/// The words the strobes write into, past the block and below the
/// device tree.
const LANES: usize = BLOCK + 4 * BLOCK_WORDS;
/// Words the strobes write into: the first half take a byte each, the
/// second half a halfword.
const LANE_WORDS: usize = 16;
/// The high words: from `0x4400_0000`, past the frame buffer, this far
/// apart, and one more in the last word of the gigabyte.
const HIGH: usize = map::DDR3 + 0x0400_0000;
const HIGH_STRIDE: usize = 0x0380_0000;
const HIGH_WORDS: usize = 16;
const TOP: usize = map::DDR3 + 0x3fff_fffc;

/// The word that goes at `addr`.
fn word(addr: usize) -> u32 {
    (addr as u32).rotate_left(7) ^ 0x5a5a_c3c3
}

/// What one check found: how many accesses were wrong, and the first.
struct Found {
    bad: u32,
    addr: usize,
    got: u32,
}

impl Found {
    fn new() -> Found {
        Found {
            bad: 0,
            addr: 0,
            got: 0,
        }
    }

    /// Notes what was read at `addr` against what should have been.
    fn check(&mut self, addr: usize, got: u32, want: u32) {
        if got != want {
            if self.bad == 0 {
                self.addr = addr;
                self.got = got;
            }
            self.bad += 1;
        }
    }

    /// Says the check's line, and whether it passed.
    fn say(&self, name: &[u8]) -> bool {
        Uart::say(b"ddr3ram ");
        Uart::say(name);
        if self.bad == 0 {
            Uart::say(b" ok\n");
            return true;
        }
        Uart::say(b" bad ");
        Uart::put_decimal(self.bad);
        Uart::say(b" at ");
        Uart::put_hex(self.addr as u32);
        Uart::say(b" read ");
        Uart::put_hex(self.got);
        Uart::put(b'\n');
        false
    }
}

fn put32(addr: usize, v: u32) {
    unsafe { write_volatile(addr as *mut u32, v) }
}
fn get32(addr: usize) -> u32 {
    unsafe { read_volatile(addr as *const u32) }
}

/// The block, written whole before any of it is read.
fn words() -> Found {
    let mut f = Found::new();
    for i in 0..BLOCK_WORDS {
        let a = BLOCK + 4 * i;
        put32(a, word(a));
    }
    for i in 0..BLOCK_WORDS {
        let a = BLOCK + 4 * i;
        f.check(a, get32(a), word(a));
    }
    f
}

/// The `i`th high word's address: `i` stretches up, and `i` times a
/// little more than a page into its stretch; the last is the top word.
fn high_at(i: usize) -> usize {
    if i == HIGH_WORDS {
        TOP
    } else {
        HIGH + i * HIGH_STRIDE + i * 0x1_0104
    }
}

/// The high words, written before any is read.
fn high() -> Found {
    let mut f = Found::new();
    for i in 0..=HIGH_WORDS {
        put32(high_at(i), word(high_at(i)));
    }
    for i in 0..=HIGH_WORDS {
        let a = high_at(i);
        f.check(a, get32(a), word(a));
    }
    f
}

/// The `k`th lane word's address, what it holds before the strobes,
/// and what it holds after: the first half take a byte at offset `k`
/// modulo four, the second half a halfword at offset two times `k`
/// modulo two.
fn lane_at(k: usize) -> usize {
    LANES + 4 * k
}
fn lane_before(k: usize) -> u32 {
    0xa5a5_a5a5 ^ (k as u32).wrapping_mul(0x0101_0101)
}
fn lane_shift(k: usize) -> u32 {
    if k < LANE_WORDS / 2 {
        8 * (k & 3) as u32
    } else {
        16 * (k & 1) as u32
    }
}
fn lane_mask(k: usize) -> u32 {
    if k < LANE_WORDS / 2 {
        0xff
    } else {
        0xffff
    }
}
fn lane_value(k: usize) -> u32 {
    (0x3c00 + 0x11 * k as u32) & lane_mask(k)
}
fn lane_after(k: usize) -> u32 {
    let mask = lane_mask(k) << lane_shift(k);
    (lane_before(k) & !mask) | (lane_value(k) << lane_shift(k))
}

/// The lane words filled, every one, then a byte or a halfword written
/// into each, then each read back whole.
fn strobes() -> Found {
    let mut f = Found::new();
    for k in 0..LANE_WORDS {
        put32(lane_at(k), lane_before(k));
    }
    for k in 0..LANE_WORDS {
        let a = lane_at(k) + (lane_shift(k) / 8) as usize;
        if k < LANE_WORDS / 2 {
            unsafe { write_volatile(a as *mut u8, lane_value(k) as u8) }
        } else {
            unsafe { write_volatile(a as *mut u16, lane_value(k) as u16) }
        }
    }
    for k in 0..LANE_WORDS {
        f.check(lane_at(k), get32(lane_at(k)), lane_after(k));
    }
    f
}

/// The lane words read again a byte at every offset and a halfword at
/// both.
fn lanes() -> Found {
    let mut f = Found::new();
    for k in 0..LANE_WORDS {
        let want = lane_after(k);
        for o in 0..4 {
            let a = lane_at(k) + o;
            let got = unsafe { read_volatile(a as *const u8) } as u32;
            f.check(a, got, (want >> (8 * o)) & 0xff);
        }
        for o in [0, 2] {
            let a = lane_at(k) + o;
            let got = unsafe { read_volatile(a as *const u16) } as u32;
            f.check(a, got, (want >> (8 * o)) & 0xffff);
        }
    }
    f
}

fn main() -> ! {
    let mut ok = words().say(b"words");
    ok &= high().say(b"high");
    ok &= strobes().say(b"strobes");
    ok &= lanes().say(b"lanes");
    Uart::say(if ok {
        b"ddr3ram ok\n"
    } else {
        b"ddr3ram bad\n"
    });
    // The verdict on a board whose serial port cannot be read: the core
    // halts only when every check passed, and the first LED shows the
    // halt. A failure spins here instead, and that LED stays dark.
    if !ok {
        loop {}
    }
    halt()
}
