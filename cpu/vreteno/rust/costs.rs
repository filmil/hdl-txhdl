// SPDX-License-Identifier: Apache-2.0
//! What an instruction costs the core on the board (issue 1561), for the
//! machine's timing mode: `board_test`'s `exclusive_costs_on_the_ddr3`
//! and `core_costs_for_the_machine`, which measure the same loops in
//! simulation, measured where the DDR3 is AMD's controller rather than
//! the simulation's model of it.
//!
//! Each kind is 256 of one instruction or pair in a loop, timed with
//! `mcycle` and less the empty loop. Every DDR3 word is in the DDR3 a
//! megabyte up, past a loaded program. Each line is `costs <kind>
//! <hundredths of a cycle an instruction>`; a host divides by a hundred.
//!
//! The instruction cache's kinds (issue 1679) are a line at a time
//! instead: `costs icache-<kind> <hundredths of a cycle a line>`, from a
//! block of straight-line code written into the DDR3 that times itself,
//! so that no fetch of this program's own is counted. The cache is
//! direct-mapped, 1024 lines of sixteen bytes.
#![no_std]
#![no_main]

use core::arch::asm;
use vreteno_hal::{entry, halt, map, Uart};

entry!(main);

/// A DDR3 word the loops use, and a stretch of 64 KiB for the loads that
/// miss, one line every 256 bytes.
const WORD: usize = map::DDR3 + 0x10_0000;
const STRETCH: usize = map::DDR3 + 0x20_0000;

/// Rounds of each loop.
const N: u32 = 256;

/// The blocks for the instruction cache. `A` starts at the cache's line
/// 512, above this program's own code; `CLASH` is 16 KiB on, so on the
/// same lines as `A`; `APART` is 18 KiB on, 2 KiB past `CLASH`, so on
/// lines `A` does not use. (16 KiB and 16 bytes on, as #1679 first had
/// it, would still share all but two of a block's 66 lines.)
const A: usize = map::DDR3 + 0x30_2000;
const CLASH: usize = A + 0x4000;
const APART: usize = A + 0x4800;

/// A block: `mcycle` read at the start of its first line, which is
/// fetched before the read and so not counted; 64 lines of 32-bit
/// no-ops; and a last line that reads `mcycle` again and returns the
/// difference. Between the reads are 65 lines: three no-ops of the
/// first, the 64, and the second read itself, which is fetched before
/// it runs. So a cold run counts 65 lines not held, and a warm one 260
/// instructions, four a line.
const NOPS: usize = 256;
const LINES: i32 = 65;
const BLOCK: usize = 4 + NOPS + 4;
// The cache is 16 KiB of 16-byte lines: `A` and `CLASH` on the same
// lines, `APART` on none of `A`'s, and this program's own code, below
// 8 KiB, on none of the blocks'.
const _: () = assert!(BLOCK * 4 <= 0x800);
const _: () = assert!((CLASH - A).is_multiple_of(0x4000));
const _: () = assert!((APART - A) % 0x4000 >= BLOCK * 4);
const _: () = assert!((A - map::DDR3) % 0x4000 >= 0x2000);
/// `csrr t0, mcycle`, `csrr t1, mcycle`, `sub a0, t1, t0`, `ret`, and the
/// 32-bit no-op, `addi x0, x0, 0`.
const CSRR_T0: u32 = 0xb000_22f3;
const CSRR_T1: u32 = 0xb000_2373;
const SUB_A0: u32 = 0x4053_0533;
const RET: u32 = 0x0000_8067;
const NOP: u32 = 0x0000_0013;

/// Rounds of the two blocks called in turn.
const ROUNDS: i32 = 8;

fn block(at: usize) {
    let code = at as *mut u32;
    for i in 0..BLOCK {
        let w = match i {
            0 => CSRR_T0,
            i if i == BLOCK - 4 => CSRR_T1,
            i if i == BLOCK - 3 => SUB_A0,
            i if i == BLOCK - 2 => RET,
            _ => NOP,
        };
        unsafe { core::ptr::write_volatile(code.add(i), w) };
    }
}

/// The block at `at` run once: the cycles it timed itself.
fn run(at: usize) -> u32 {
    let f: extern "C" fn() -> u32 = unsafe { core::mem::transmute(at) };
    f()
}

/// `fence`, which waits for the stores the core has posted, then
/// `fence.i`, which empties the instruction cache, so that what is
/// fetched next is what the stores wrote.
fn fence_i() {
    unsafe { asm!("fence", ".word 0x0000100f") };
}

/// The `A` block and `other` called in turn, both warm first: the
/// cycles of all `2 * ROUNDS` calls.
fn in_turn(other: usize) -> i32 {
    run(A);
    run(other);
    let mut sum = 0i32;
    for _ in 0..ROUNDS {
        sum += run(A) as i32 + run(other) as i32;
    }
    sum
}

/// `costs icache-<name> <hundredths of a cycle a line>`, for `cycles`
/// over `calls` runs of a block, signed, since a figure that should be
/// nothing may come out a little below it.
fn say_line(name: &[u8], cycles: i32, calls: i32) {
    let per = cycles * 100 / (calls * LINES);
    Uart::say(b"costs icache-");
    Uart::say(name);
    Uart::put(b' ');
    if per < 0 {
        Uart::put(b'-');
    }
    Uart::put_decimal(per.unsigned_abs());
    Uart::put(b'\n');
}

/// Whether the block at `at` was written: its `ret` read back.
fn written(at: usize) -> bool {
    let last = (at as *const u32).wrapping_add(BLOCK - 2);
    unsafe { core::ptr::read_volatile(last) == RET }
}

/// The instruction cache's kinds, with the raw cycles beside them:
/// `icache cold <c> warm <w> clash <x> apart <y>`, where `x` and `y`
/// are the cycles of all `2 * ROUNDS` calls.
fn icache() {
    block(A);
    block(CLASH);
    block(APART);
    // The stores reach the DDR3 and not the instruction cache: a fence
    // waits for them, and `fence.i` empties the cache.
    fence_i();
    if !written(A) || !written(CLASH) || !written(APART) {
        Uart::say(b"costs icache blocks not written\n");
        return;
    }
    fence_i();
    let cold = run(A) as i32;
    let warm = run(A) as i32;
    let clash = in_turn(CLASH);
    let apart = in_turn(APART);
    Uart::say(b"icache cold ");
    Uart::put_decimal(cold as u32);
    Uart::say(b" warm ");
    Uart::put_decimal(warm as u32);
    Uart::say(b" clash ");
    Uart::put_decimal(clash as u32);
    Uart::say(b" apart ");
    Uart::put_decimal(apart as u32);
    Uart::put(b'\n');
    // A line not held, from the cold run and from the clash; a warm
    // line, which is four cycles if running on into the next line costs
    // nothing; and the blocks apart, which should cost nothing more.
    let calls = 2 * ROUNDS;
    say_line(b"miss", cold - warm, 1);
    say_line(b"clash", clash - calls * warm, calls);
    say_line(b"warm", warm, 1);
    say_line(b"apart", apart - calls * warm, calls);
}

fn mcycle() -> u32 {
    let c: u32;
    unsafe { asm!("csrr {0}, mcycle", out(reg) c) };
    c
}

/// The cycles of `N` rounds of the loop whose body is `$body`, with `a0`
/// the word's address, `a1` the stretch's, `a2` and `a3` scratch, and a
/// fence after so that nothing is left on its way.
macro_rules! timed {
    ($($body:literal),*) => {{
        let t0 = mcycle();
        unsafe {
            asm!(
                ".option push",
                ".option arch, +a",
                "1:",
                $($body,)*
                "addi {n}, {n}, -1",
                "bnez {n}, 1b",
                "fence",
                ".option pop",
                n = inout(reg) N => _,
                in("a0") WORD,
                inout("a1") STRETCH => _,
                out("a2") _,
                out("a3") _,
            )
        };
        mcycle().wrapping_sub(t0)
    }};
}

fn say(name: &[u8], cycles: u32, empty: u32) {
    // Hundredths of a cycle an instruction: 100 / 256 is 25 / 64.
    let per = (cycles.wrapping_sub(empty) * 25) >> 6;
    Uart::say(b"costs ");
    Uart::say(name);
    Uart::put(b' ');
    Uart::put_decimal(per);
    Uart::put(b'\n');
}

fn main() -> ! {
    // The word in the data cache, and a divisor.
    unsafe { core::ptr::write_volatile(WORD as *mut u32, 7) };
    let _ = unsafe { core::ptr::read_volatile(WORD as *const u32) };
    let empty = timed!();
    say(b"load", timed!("lw a2, 0(a0)"), empty);
    say(b"store", timed!("sw a2, 0(a0)"), empty);
    say(b"amoadd.w", timed!("amoadd.w a2, a2, (a0)"), empty);
    say(
        b"lr.w+sc.w",
        timed!("lr.w a2, (a0)", "sc.w a3, a2, (a0)"),
        empty,
    );
    say(b"lr.w", timed!("lr.w a2, (a0)"), empty);
    say(b"fence", timed!("fence"), empty);
    say(b"store+fence", timed!("sw a2, 0(a0)", "fence"), empty);
    say(
        b"load-miss+step",
        timed!("lw a2, 0(a1)", "addi a1, a1, 256"),
        empty,
    );
    say(b"mul", timed!("mul a2, a2, a2"), empty);
    say(b"divu", timed!("lw a3, 0(a0)", "divu a2, a2, a3"), empty);
    icache();
    Uart::say(b"costs done\n");
    halt()
}
