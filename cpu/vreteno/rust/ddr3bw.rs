// SPDX-License-Identifier: Apache-2.0
//! The path into DDR3 timed by the core, on the board: the baseline of
//! issue 1023, which `//ddr3:bw` measures in simulation.
//!
//! Sixteen loads, then sixteen stores and a fence, each run from the
//! data memory and timed with `mcycle`, once against the DDR3 and once
//! against the data memory itself, four times each. The routines run
//! from block RAM, as `steps.rs`'s does, so that their fetch takes the
//! same time whichever memory they touch: a loaded program runs from
//! the DDR3, and a loop fetched from there would time its own fetch.
//! What the two memories' lines differ by, divided by sixteen, is what a
//! word costs the DDR3's path over the block RAM's.
//!
//! The core has one load out at a time, so a load waits for its word:
//! the loads give the path's round trip. The stores are posted, and the
//! fence waits until every one is answered: the stores give how fast the
//! path takes words when the core does not wait for each.
//!
//! Each line is `bw <load|store> <ddr3|dmem> <cycles>`, the cycles for
//! sixteen words. A host divides; nothing here divides, so nothing asks
//! for `core`'s panic path.
#![no_std]
#![no_main]

use core::ptr::{read_volatile, write_volatile};
use vreteno_hal::{entry, halt, map, Uart};

/// Where the two routines go, as for `steps.rs`: up the data memory,
/// above what a program built into the boot memory keeps there and
/// below its stack.
const LOADS_AT: usize = map::DMEM + 0x800;
const STORES_AT: usize = map::DMEM + 0x880;
/// The words the routines touch in the data memory, past the routines.
const DMEM_WORDS: usize = map::DMEM + 0x900;
/// The words they touch in the DDR3: a megabyte up, past a loaded
/// program, its data and its stack.
const DDR3_WORDS: usize = map::DDR3 + 0x10_0000;

/// Words a routine touches.
const N: usize = 16;
/// Times each routine runs against each memory.
const REPS: u32 = 4;

/// Registers, by number: the argument and result, and three
/// temporaries.
const A0: u32 = 10;
const T0: u32 = 5;
const T1: u32 = 6;
const T2: u32 = 7;
const ZERO: u32 = 0;

/// `csrr rd, mcycle`.
const fn csrr_mcycle(rd: u32) -> u32 {
    0xb000_2073 | (rd << 7)
}
/// `lw rd, imm(rs1)`.
const fn lw(rd: u32, rs1: u32, imm: u32) -> u32 {
    (imm << 20) | (rs1 << 15) | (2 << 12) | (rd << 7) | 0x03
}
/// `sw rs2, imm(rs1)`.
const fn sw(rs2: u32, rs1: u32, imm: u32) -> u32 {
    ((imm >> 5) << 25)
        | (rs2 << 20)
        | (rs1 << 15)
        | (2 << 12)
        | ((imm & 31) << 7)
        | 0x23
}
/// `sub a0, t2, t0`: the second reading less the first.
const SUB_A0_T2_T0: u32 = 0x4053_8533;
/// `fence`.
const FENCE: u32 = 0x0ff0_000f;
/// `ret`.
const RET: u32 = 0x0000_8067;

/// Words of a routine: the two readings, the accesses, the difference
/// and the return, and the stores' fence.
const LEN: usize = N + 5;

/// The loads: `a0` is where, and comes back as the cycles.
const fn loads() -> [u32; LEN] {
    let mut r = [0u32; LEN];
    r[0] = csrr_mcycle(T0);
    let mut i = 0;
    while i < N {
        r[1 + i] = lw(T1, A0, (4 * i) as u32);
        i += 1;
    }
    r[N + 1] = csrr_mcycle(T2);
    r[N + 2] = SUB_A0_T2_T0;
    r[N + 3] = RET;
    r[N + 4] = RET;
    r
}

/// The stores of zero, and a fence that waits for them all.
const fn stores() -> [u32; LEN] {
    let mut r = [0u32; LEN];
    r[0] = csrr_mcycle(T0);
    let mut i = 0;
    while i < N {
        r[1 + i] = sw(ZERO, A0, (4 * i) as u32);
        i += 1;
    }
    r[N + 1] = FENCE;
    r[N + 2] = csrr_mcycle(T2);
    r[N + 3] = SUB_A0_T2_T0;
    r[N + 4] = RET;
    r
}

const LOADS: [u32; LEN] = loads();
const STORES: [u32; LEN] = stores();

entry!(main);

/// A routine written into the data memory at `at`.
fn place(at: usize, words: &[u32; LEN]) {
    let code = at as *mut u32;
    for (i, w) in words.iter().enumerate() {
        unsafe { write_volatile(code.add(i), *w) };
    }
}

/// A routine run `REPS` times against `words`, and each run said.
fn time(at: usize, what: &[u8], words: usize) {
    let routine: extern "C" fn(usize) -> u32 =
        unsafe { core::mem::transmute(at) };
    for _ in 0..REPS {
        let cycles = routine(words);
        Uart::say(b"bw ");
        Uart::say(what);
        Uart::put(b' ');
        Uart::put_decimal(cycles);
        Uart::put(b'\n');
    }
}

fn main() -> ! {
    place(LOADS_AT, &LOADS);
    place(STORES_AT, &STORES);
    // The stores are posted; a fence waits for them, and there is no
    // instruction cache, so the routines are in place when fetched.
    unsafe { core::arch::asm!("fence") };
    if unsafe { read_volatile((STORES_AT as *const u32).add(LEN - 1)) } != RET {
        Uart::say(b"bw routines not written\n");
        halt()
    }
    time(LOADS_AT, b"load ddr3", DDR3_WORDS);
    time(LOADS_AT, b"load dmem", DMEM_WORDS);
    time(STORES_AT, b"store ddr3", DDR3_WORDS);
    time(STORES_AT, b"store dmem", DMEM_WORDS);
    halt()
}
