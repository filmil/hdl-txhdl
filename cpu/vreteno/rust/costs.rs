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

/// Four more blocks for the instruction cache (issue 1679). `FAR`, `NEAR`
/// and `LOOP` each time the same 260 instructions as `A`, so that four
/// instructions of one compare with four of another; `HELD` runs three a
/// line, 195. `FAR`, `NEAR` and `HELD` are over 65 lines as `A`, `LOOP`
/// in one:
///
/// * `FAR`: 64 lines 128 bytes apart, run in the order `37 k mod 64`,
///   each three no-ops and a `jal` to the next, so that no line follows
///   the one before it and a fill cannot start early.
/// * `NEAR`: `A`'s lines, each ending in a `jal` to the next line rather
///   than running off its end.
/// * `LOOP`: four instructions in one line, the last a taken branch back,
///   run 64 times: a line's instructions with no line crossed.
/// * `HELD`: `FAR` with each line's `jal` one word earlier, so that the
///   word after it is in its own line. The core fetches the word after a
///   jump while the jump executes; in `FAR` that word starts a line not
///   held, and its fill may go out before the target's.
const FAR: usize = map::DDR3 + 0x31_2000;
const NEAR: usize = map::DDR3 + 0x32_2000;
const LOOP: usize = map::DDR3 + 0x33_2000;
const HELD: usize = map::DDR3 + 0x34_2000;
const FAR_STEP: usize = 128;
// Each on lines 512 and up, which this program's code does not reach,
// and `FAR`'s header and last line on lines its 64 do not use, away
// from the first line it runs.
const _: () = assert!((FAR - map::DDR3) % 0x4000 == 0x2000);
const _: () = assert!((NEAR - map::DDR3) % 0x4000 == 0x2000);
const _: () = assert!((LOOP - map::DDR3) % 0x4000 == 0x2000);
const _: () = assert!((HELD - map::DDR3) % 0x4000 == 0x2000);
const _: () = assert!(64 * FAR_STEP <= 0x2000);

/// `jal x0, off`, for an even offset of 21 bits.
const fn jal(off: i32) -> u32 {
    let o = off as u32;
    (((o >> 20) & 1) << 31)
        | (((o >> 1) & 0x3ff) << 21)
        | (((o >> 11) & 1) << 20)
        | (((o >> 12) & 0xff) << 12)
        | 0x6f
}
/// `addi t2, zero, 64`, `addi t2, t2, -1` and `bne t2, zero, -12`.
const LI_T2: u32 = 0x0400_0393;
const DEC_T2: u32 = 0xfff3_8393;
const BNE_T2: u32 = 0xfe03_9ae3;

fn put(at: usize, words: [u32; 4]) {
    let code = at as *mut u32;
    for (i, w) in words.iter().enumerate() {
        unsafe { core::ptr::write_volatile(code.add(i), *w) };
    }
}

/// `FAR`'s header and last line: half-way into the 128 bytes of the line
/// it runs 32nd, far from the first it runs, at `FAR`, and from the last.
const FAR_HEAD: usize = FAR + 32 * FAR_STEP + 64;
const FAR_END: usize = FAR_HEAD + 16;

/// `FAR`: the header, the 64 lines at `FAR + 128 p`, and the last line.
fn far() {
    let line = |k: usize| FAR + FAR_STEP * ((k * 37) % 64);
    let end = FAR_END;
    let hop = |from: usize, to: usize| jal(to as i32 - (from + 12) as i32);
    put(FAR_HEAD, [CSRR_T0, NOP, NOP, hop(FAR_HEAD, line(0))]);
    for k in 0..64 {
        let next = if k == 63 { end } else { line(k + 1) };
        put(line(k), [NOP, NOP, NOP, hop(line(k), next)]);
    }
    put(end, [CSRR_T1, SUB_A0, RET, NOP]);
}

/// `HELD`'s header and last line, placed as `FAR`'s.
const HELD_HEAD: usize = HELD + 32 * FAR_STEP + 64;
const HELD_END: usize = HELD_HEAD + 16;

/// `HELD`: as `FAR`, with each `jal` in its line's third word and a
/// no-op after it, which is fetched and never run.
fn held() {
    let line = |k: usize| HELD + FAR_STEP * ((k * 37) % 64);
    let end = HELD_END;
    let hop = |from: usize, to: usize| jal(to as i32 - (from + 8) as i32);
    put(HELD_HEAD, [CSRR_T0, NOP, hop(HELD_HEAD, line(0)), NOP]);
    for k in 0..64 {
        let next = if k == 63 { end } else { line(k + 1) };
        put(line(k), [NOP, NOP, hop(line(k), next), NOP]);
    }
    put(end, [CSRR_T1, SUB_A0, RET, NOP]);
}

/// `NEAR`: as `A`, with each line's last word a `jal` to the next line.
fn near() {
    put(NEAR, [CSRR_T0, NOP, NOP, jal(4)]);
    for l in 1..65 {
        put(NEAR + 16 * l, [NOP, NOP, NOP, jal(4)]);
    }
    put(NEAR + 16 * 65, [CSRR_T1, SUB_A0, RET, NOP]);
}

/// `LOOP`: a header that sets `t2` to 64, the loop's one line, and the
/// last line.
fn one_line_loop() {
    put(LOOP, [CSRR_T0, LI_T2, NOP, NOP]);
    put(LOOP + 16, [DEC_T2, NOP, NOP, BNE_T2]);
    put(LOOP + 32, [CSRR_T1, SUB_A0, RET, NOP]);
}

/// The four blocks, each run cold after `fence.i` and then warm:
/// `icache2 far <cold> <warm> near <cold> <warm> loop <cold> <warm>
/// held <cold> <warm>`, then `costs icache-<kind>` a line, as for `A`.
fn icache_jumps() {
    far();
    near();
    one_line_loop();
    held();
    fence_i();
    // Each block's `ret`, the third word of its last line, read back.
    for end in [FAR_END, NEAR + 16 * 65, LOOP + 32, HELD_END] {
        let ret = (end as *const u32).wrapping_add(2);
        if unsafe { core::ptr::read_volatile(ret) != RET } {
            Uart::say(b"costs icache jump blocks not written\n");
            return;
        }
    }
    let mut cold = [0i32; 4];
    let mut warm = [0i32; 4];
    for (i, at) in [FAR_HEAD, NEAR, LOOP, HELD_HEAD].into_iter().enumerate() {
        fence_i();
        cold[i] = run(at) as i32;
        warm[i] = run(at) as i32;
    }
    Uart::say(b"icache2");
    let names = [&b" far "[..], b" near ", b" loop ", b" held "];
    for (i, name) in names.iter().enumerate() {
        Uart::say(name);
        Uart::put_decimal(cold[i] as u32);
        Uart::put(b' ');
        Uart::put_decimal(warm[i] as u32);
    }
    Uart::put(b'\n');
    // A far line not held, beside its warm cost; a near jump's warm and
    // missed line; and four instructions in one line, no line crossed.
    say_line(b"far-miss", cold[0] - warm[0], 1);
    say_line(b"far-warm", warm[0], 1);
    say_line(b"near-miss", cold[1] - warm[1], 1);
    say_line(b"near-warm", warm[1], 1);
    say_line(b"loop-warm", warm[2], 1);
    // A far line not held whose jump's next word is in its own line, and
    // its warm cost, which is of three instructions a line, not four.
    say_line(b"held-miss", cold[3] - warm[3], 1);
    say_line(b"held-warm", warm[3], 1);
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
    icache_jumps();
    Uart::say(b"costs done\n");
    halt()
}
