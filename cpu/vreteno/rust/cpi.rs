// SPDX-License-Identifier: Apache-2.0
//! The instruction cache timed by the core, on the board (issue 1320):
//! `//cpu/vreteno:icache_test`'s loop, which that test measures only in
//! simulation.
//!
//! The loop is three instructions a thousand times round. It is written
//! into the data memory and into the DDR3, the two memories the cache
//! holds, and run from each twice: cold, with a `fence.i` inside the
//! time, which clears the tags a line a cycle and leaves every line to
//! be filled again, as `icache_test` counts from the reset; and warm,
//! straight after, when the loop's line is in the cache.
//!
//! Each line is `cpi <dmem|ddr3> <cold|warm> <cycles> <instret>`, from
//! `mcycle` and `minstret` read around the loop. A host divides; nothing
//! here divides, so nothing asks for `core`'s panic path.
#![no_std]
#![no_main]

use core::ptr::{read_volatile, write_volatile};
use vreteno_hal::{entry, halt, map, Uart};

/// Where the routines go in the data memory: up it, above what a
/// program built into the boot memory keeps there and below its stack,
/// as for `ddr3bw.rs`.
const DMEM_COLD: usize = map::DMEM + 0x800;
const DMEM_WARM: usize = map::DMEM + 0x880;
/// And in the DDR3: a megabyte up, past a loaded program, its data and
/// its stack.
const DDR3_COLD: usize = map::DDR3 + 0x10_0000;
const DDR3_WARM: usize = map::DDR3 + 0x10_0080;

/// Rounds of the loop.
const ROUNDS: u32 = 1000;

/// Registers, by number: the results and five temporaries.
const A0: u32 = 10;
const A1: u32 = 11;
const T0: u32 = 5;
const T1: u32 = 6;
const T2: u32 = 7;
const T3: u32 = 28;
const T4: u32 = 29;
const T5: u32 = 30;
const ZERO: u32 = 0;

/// `csrr rd, mcycle`.
const fn csrr_mcycle(rd: u32) -> u32 {
    0xb000_2073 | (rd << 7)
}
/// `csrr rd, minstret`.
const fn csrr_minstret(rd: u32) -> u32 {
    0xb020_2073 | (rd << 7)
}
/// `addi rd, rs1, imm`, for an immediate of twelve bits.
const fn addi(rd: u32, rs1: u32, imm: i32) -> u32 {
    (((imm as u32) & 0xfff) << 20) | (rs1 << 15) | (rd << 7) | 0x13
}
/// `bne rs1, rs2, off`, for an even offset of thirteen bits.
const fn bne(rs1: u32, rs2: u32, off: i32) -> u32 {
    let o = off as u32;
    (((o >> 12) & 1) << 31)
        | (((o >> 5) & 0x3f) << 25)
        | (rs2 << 20)
        | (rs1 << 15)
        | (1 << 12)
        | (((o >> 1) & 0xf) << 8)
        | (((o >> 11) & 1) << 7)
        | 0x63
}
/// `sub rd, rs1, rs2`.
const fn sub(rd: u32, rs1: u32, rs2: u32) -> u32 {
    0x4000_0033 | (rs2 << 20) | (rs1 << 15) | (rd << 7)
}
/// `fence.i`, and `addi zero, zero, 0`, which pads the warm routine to
/// the cold one's length.
const FENCE_I: u32 = 0x0000_100f;
const NOP: u32 = 0x0000_0013;
/// `ret`.
const RET: u32 = 0x0000_8067;

/// Words of a routine.
const LEN: usize = 13;

/// The routine: the two counters read, a `fence.i` when cold, the loop,
/// the counters read again, and the differences in `a0` and `a1`.
const fn routine(cold: bool) -> [u32; LEN] {
    [
        csrr_mcycle(T0),
        csrr_minstret(T3),
        if cold { FENCE_I } else { NOP },
        addi(T2, ZERO, ROUNDS as i32),
        addi(T1, T1, 1),
        addi(T2, T2, -1),
        bne(T2, ZERO, -8),
        csrr_mcycle(T4),
        csrr_minstret(T5),
        sub(A0, T4, T0),
        sub(A1, T5, T3),
        RET,
        RET,
    ]
}

const COLD: [u32; LEN] = routine(true);
const WARM: [u32; LEN] = routine(false);

entry!(main);

/// A routine written at `at`.
fn place(at: usize, words: &[u32; LEN]) {
    let code = at as *mut u32;
    for (i, w) in words.iter().enumerate() {
        unsafe { write_volatile(code.add(i), *w) };
    }
}

/// The routine at `at` run once, and what it timed said.
fn time(at: usize, what: &[u8]) {
    let routine: extern "C" fn() -> u64 = unsafe { core::mem::transmute(at) };
    let both = routine();
    Uart::say(b"cpi ");
    Uart::say(what);
    Uart::put(b' ');
    Uart::put_decimal(both as u32);
    Uart::put(b' ');
    Uart::put_decimal((both >> 32) as u32);
    Uart::put(b'\n');
}

fn main() -> ! {
    place(DMEM_COLD, &COLD);
    place(DMEM_WARM, &WARM);
    place(DDR3_COLD, &COLD);
    place(DDR3_WARM, &WARM);
    // The stores are posted and do not reach the cache: a fence waits
    // for them, and a `fence.i` makes what was written what is fetched.
    unsafe { core::arch::asm!("fence", ".word 0x0000100f") };
    if unsafe { read_volatile((DDR3_WARM as *const u32).add(LEN - 1)) } != RET {
        Uart::say(b"cpi routines not written\n");
        halt()
    }
    time(DMEM_COLD, b"dmem cold");
    time(DMEM_WARM, b"dmem warm");
    time(DDR3_COLD, b"ddr3 cold");
    time(DDR3_WARM, b"ddr3 warm");
    halt()
}
