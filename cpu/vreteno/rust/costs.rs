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
//!
//! The bulk kinds are a megabyte read, set to zero and copied, a word at
//! a time as Linux's `memset` and `memcpy` go, from memory no cache
//! holds: `costs bulk-<kind> <hundredths of a cycle a word>`. Linux's
//! boot on the board spends its early part setting memory and its
//! framebuffer console copying it, and the model is wrong in opposite
//! directions there (issue 1561). The `scan-bulk` kinds are the same
//! with the scanout reading a frame from the DDR3 beside them, as it
//! does through Linux's boot on the board, and `scan-icache-miss` is
//! `icache-miss` with the scanout showing.
#![no_std]
#![no_main]

use core::arch::asm;
use core::sync::atomic::{AtomicU32, Ordering};
use vreteno_hal::{entry, halt, map, Hart1, Scan, Uart};

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
    Uart::say(b"costs icache-");
    Uart::say(name);
    put_line(cycles, calls);
}

/// A space, then `cycles` over `calls` runs of a block in hundredths of a
/// cycle a line, signed, and the line's end.
fn put_line(cycles: i32, calls: i32) {
    let per = cycles * 100 / (calls * LINES);
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

/// `GUESS` (#1752): `LOOP` with its second word a backward branch never
/// taken, `bne zero, zero, -4`. The core guesses a backward branch taken
/// and corrects the guess a cycle later, so each round pays for a wrong
/// guess that `LOOP`'s round does not.
const GUESS: usize = map::DDR3 + 0x35_2000;
const _: () = assert!((GUESS - map::DDR3) % 0x4000 == 0x2000);
const BNE_NEVER: u32 = 0xfe00_1ee3;

fn guess_loop() {
    put(GUESS, [CSRR_T0, LI_T2, NOP, NOP]);
    put(GUESS + 16, [DEC_T2, BNE_NEVER, NOP, BNE_T2]);
    put(GUESS + 32, [CSRR_T1, SUB_A0, RET, NOP]);
}

/// Whether hart 1 streams its loads, and whether it has stopped: 0 not
/// started, 1 streaming, 2 told to stop, 3 stopped.
static STREAM: AtomicU32 = AtomicU32::new(0);

/// Hart 1's loads for `icache_contend`: one a line over the 64 KiB
/// stretch, every one a miss in its data cache of 4 KiB, until hart 0
/// says stop.
extern "C" fn stream(_: u32, _: u32) -> ! {
    STREAM.store(1, Ordering::Release);
    while STREAM.load(Ordering::Acquire) == 1 {
        let mut at = STRETCH;
        while at < STRETCH + 0x1_0000 {
            let _ = unsafe { core::ptr::read_volatile(at as *const u32) };
            at += 16;
        }
    }
    STREAM.store(3, Ordering::Release);
    Hart1::park()
}

/// A branch guessed wrong, and a line not held while the other hart
/// reads the DDR3 (#1752): `icache3 guess <cold> <warm> contend <cold>
/// <warm>`, then `costs icache-guess-warm`, and `costs
/// icache-contend-miss` beside `icache-miss`.
fn icache_guess_contend() {
    guess_loop();
    block(A);
    fence_i();
    let ret = (GUESS + 32 + 8) as *const u32;
    if unsafe { core::ptr::read_volatile(ret) != RET } || !written(A) {
        Uart::say(b"costs icache guess blocks not written\n");
        return;
    }
    fence_i();
    let g_cold = run(GUESS) as i32;
    let g_warm = run(GUESS) as i32;
    Hart1::start(stream, 0);
    while STREAM.load(Ordering::Acquire) != 1 {}
    fence_i();
    let c_cold = run(A) as i32;
    let c_warm = run(A) as i32;
    STREAM.store(2, Ordering::Release);
    while STREAM.load(Ordering::Acquire) != 3 {}
    Uart::say(b"icache3 guess ");
    Uart::put_decimal(g_cold as u32);
    Uart::put(b' ');
    Uart::put_decimal(g_warm as u32);
    Uart::say(b" contend ");
    Uart::put_decimal(c_cold as u32);
    Uart::put(b' ');
    Uart::put_decimal(c_warm as u32);
    Uart::put(b'\n');
    say_line(b"guess-warm", g_warm, 1);
    say_line(b"contend-miss", c_cold - c_warm, 1);
    say_line(b"contend-warm", c_warm, 1);
}

/// Four more blocks for #1752, each against one it differs from in one
/// thing:
///
/// * `FWD` and `JMP`: `LOOP` with its second word skipping its third,
///   by a forward branch taken, `beq zero, zero, 8`, which the core
///   guesses not taken, or by `jal zero, 8`. Their difference is a wrong
///   forward guess.
/// * `GUESS2`: `GUESS` with its branch's target a line back, the
///   header's, which is held, where `GUESS`'s is its own line. Their
///   difference is what the wrong path's lookup of another line costs.
/// * `SCATTER`: `HELD`'s 64 lines, each `jal` its line's third word,
///   with the lines 16 KiB and 128 bytes apart, so that each is in a
///   DDR3 row of its own, where `HELD`'s 64 share two. Their difference,
///   cold, is what a fill costs when it opens a row.
const FWD: usize = map::DDR3 + 0x36_2000;
const JMP: usize = map::DDR3 + 0x37_2000;
const GUESS2: usize = map::DDR3 + 0x38_2000;
const SCATTER: usize = map::DDR3 + 0x40_2000;
const SCATTER_STEP: usize = 0x4080;
const _: () = assert!((FWD - map::DDR3) % 0x4000 == 0x2000);
const _: () = assert!((JMP - map::DDR3) % 0x4000 == 0x2000);
const _: () = assert!((GUESS2 - map::DDR3) % 0x4000 == 0x2000);
const _: () = assert!((SCATTER - map::DDR3) % 0x4000 == 0x2000);
/// `beq zero, zero, 8`; `jal zero, 8`; `bne zero, zero, -20`.
const BEQ_SKIP: u32 = 0x0000_0463;
const JAL_SKIP: u32 = 0x0080_006f;
const BNE_LINE_BACK: u32 = 0xfe00_16e3;

/// `SCATTER`'s header and last line: in the 128 bytes past its first
/// line, on lines none of its 64 use.
const SCATTER_HEAD: usize = SCATTER + 64;
const SCATTER_END: usize = SCATTER_HEAD + 16;

fn icache4_blocks() {
    for (at, w1) in [(FWD, BEQ_SKIP), (JMP, JAL_SKIP), (GUESS2, BNE_LINE_BACK)]
    {
        put(at, [CSRR_T0, LI_T2, NOP, NOP]);
        put(at + 16, [DEC_T2, w1, NOP, BNE_T2]);
        put(at + 32, [CSRR_T1, SUB_A0, RET, NOP]);
    }
    let line = |k: usize| SCATTER + SCATTER_STEP * k;
    let hop = |from: usize, to: usize| jal(to as i32 - (from + 8) as i32);
    put(
        SCATTER_HEAD,
        [CSRR_T0, NOP, hop(SCATTER_HEAD, line(0)), NOP],
    );
    for k in 0..64 {
        let next = if k == 63 { SCATTER_END } else { line(k + 1) };
        put(line(k), [NOP, NOP, hop(line(k), next), NOP]);
    }
    put(SCATTER_END, [CSRR_T1, SUB_A0, RET, NOP]);
}

/// The four, each cold after `fence.i` and then warm: `icache4 fwd <cold>
/// <warm> jmp <cold> <warm> guess2 <cold> <warm> scatter <cold> <warm>`,
/// then `costs icache-<kind>` a line.
fn icache4() {
    icache4_blocks();
    fence_i();
    let ends = [FWD + 32, JMP + 32, GUESS2 + 32, SCATTER_END];
    for end in ends {
        let ret = (end as *const u32).wrapping_add(2);
        if unsafe { core::ptr::read_volatile(ret) != RET } {
            Uart::say(b"costs icache4 blocks not written\n");
            return;
        }
    }
    let mut cold = [0i32; 4];
    let mut warm = [0i32; 4];
    for (i, at) in [FWD, JMP, GUESS2, SCATTER_HEAD].into_iter().enumerate() {
        fence_i();
        cold[i] = run(at) as i32;
        warm[i] = run(at) as i32;
    }
    Uart::say(b"icache4");
    let names = [&b" fwd "[..], b" jmp ", b" guess2 ", b" scatter "];
    for (i, name) in names.iter().enumerate() {
        Uart::say(name);
        Uart::put_decimal(cold[i] as u32);
        Uart::put(b' ');
        Uart::put_decimal(warm[i] as u32);
    }
    Uart::put(b'\n');
    say_line(b"fwd-warm", warm[0], 1);
    say_line(b"jmp-warm", warm[1], 1);
    say_line(b"guess2-warm", warm[2], 1);
    say_line(b"scatter-miss", cold[3] - warm[3], 1);
    say_line(b"scatter-warm", warm[3], 1);
}

/// Four loops of two lines for #1918, each the shape of `ethperf.rs`'s
/// copy into a transmit slot: six compressed instructions, the first two
/// at the end of one line and the other four at the start of the next,
/// the last a taken branch back whose next word is in its own line. The
/// model charges ethperf's copy 17 cycles a round where the board takes
/// 12, and the blocks already here cannot say which charge is wrong:
///
/// * `CROSS`: no-ops, so a round is the crossing, the branch and the
///   counter alone.
/// * `ST_CROSS`: a store into the DDR3 just before the crossing.
/// * `CROSS_ST`: the crossing landing on the store.
/// * `COPY`: `copy_in`'s loop exactly, a load the data cache holds and
///   a store into the DDR3 a round, with the crossing on the store.
///
/// Each runs 64 rounds, cold and then warm, the warm one reported:
/// `icache5 cross <cold> <warm> st-cross .. cross-st .. copy ..`, then
/// `costs loop2-<kind> <hundredths of a cycle a round>`.
const CROSS: usize = map::DDR3 + 0x39_2000;
const ST_CROSS: usize = map::DDR3 + 0x3a_2000;
const CROSS_ST: usize = map::DDR3 + 0x3b_2000;
const COPY: usize = map::DDR3 + 0x3c_2000;
const _: () = assert!((CROSS - map::DDR3) % 0x4000 == 0x2000);
const _: () = assert!((ST_CROSS - map::DDR3) % 0x4000 == 0x2000);
const _: () = assert!((CROSS_ST - map::DDR3) % 0x4000 == 0x2000);
const _: () = assert!((COPY - map::DDR3) % 0x4000 == 0x2000);
/// `addi a1, zero, 64`; `lui a2, 0x40101`, the copy's source, a page past
/// `WORD`; `lui a5, 0x40102`, where every store goes, a page further.
const LI_A1: u32 = 0x0400_0593;
const LUI_A2: u32 = 0x4010_1637;
const LUI_A5: u32 = 0x4010_27b7;
/// The compressed instructions, as `ethperf.rs` compiles to them:
/// `c.lw a4, 0(a2)`, `c.addi a2, 4`, `c.sw a4, 0(a5)`, `c.addi a5, 4`,
/// `c.addi a1, -1`, `c.bnez a1, -10` and `c.nop`.
const C_LW: u32 = 0x4218;
const C_ADD_A2: u32 = 0x0611;
const C_SW: u32 = 0xc398;
const C_ADD_A5: u32 = 0x0791;
const C_DEC_A1: u32 = 0x15fd;
const C_BNEZ_A1: u32 = 0xf9fd;
const C_NOP: u32 = 0x0001;

/// Two compressed instructions in one word, the first at the lower half.
const fn pair(lo: u32, hi: u32) -> u32 {
    lo | (hi << 16)
}

/// A loop of two lines at `at`: the header, three no-ops, then `a` and
/// `b` in the first line's last word, and `c`, `d`, the counter and the
/// branch in the second line's first two words, which falls through to
/// the end.
fn loop2(at: usize, [a, b, c, d]: [u32; 4]) {
    put(at, [CSRR_T0, LI_A1, LUI_A2, LUI_A5]);
    put(at + 16, [NOP, NOP, NOP, pair(a, b)]);
    put(
        at + 32,
        [pair(c, d), pair(C_DEC_A1, C_BNEZ_A1), CSRR_T1, SUB_A0],
    );
    put(at + 48, [RET, NOP, NOP, NOP]);
}

/// The four loops of two lines (#1918).
fn icache5() {
    loop2(CROSS, [C_NOP, C_NOP, C_NOP, C_NOP]);
    loop2(ST_CROSS, [C_NOP, C_SW, C_NOP, C_NOP]);
    loop2(CROSS_ST, [C_NOP, C_NOP, C_SW, C_NOP]);
    loop2(COPY, [C_LW, C_ADD_A2, C_SW, C_ADD_A5]);
    fence_i();
    for at in [CROSS, ST_CROSS, CROSS_ST, COPY] {
        let ret = (at + 48) as *const u32;
        if unsafe { core::ptr::read_volatile(ret) != RET } {
            Uart::say(b"costs loop2 blocks not written\n");
            return;
        }
    }
    let names = [&b"cross"[..], b"st-cross", b"cross-st", b"copy"];
    let mut cold = [0u32; 4];
    let mut warm = [0u32; 4];
    for (i, at) in [CROSS, ST_CROSS, CROSS_ST, COPY].into_iter().enumerate() {
        fence_i();
        cold[i] = run(at);
        warm[i] = run(at);
    }
    Uart::say(b"icache5");
    for (i, name) in names.iter().enumerate() {
        Uart::put(b' ');
        Uart::say(name);
        Uart::put(b' ');
        Uart::put_decimal(cold[i]);
        Uart::put(b' ');
        Uart::put_decimal(warm[i]);
    }
    Uart::put(b'\n');
    for (i, name) in names.iter().enumerate() {
        Uart::say(b"costs loop2-");
        Uart::say(name);
        Uart::put(b' ');
        Uart::put_decimal(warm[i] * 100 / 64);
        Uart::put(b'\n');
    }
}

/// Five more loops for #1918, which `icache5`'s four could not tell
/// apart. Those cost the crossing and the branch back about 5 together,
/// where the model charges 7, and the copy's load and store 1, where it
/// charges 4: the board seems to overlap the fetch's stalls and the
/// data's, which the model adds. These separate the two:
///
/// * `ONE`: the same six instructions in one line, the branch's target
///   at its start, so a round has no crossing.
/// * `ONE_LW`: a load the data cache holds, in one line.
/// * `ONE_SW`: a store into the DDR3, in one line.
/// * `ONE_COPY`: `copy_in`'s loop in one line.
/// * `TWO_LW`: the load alone in the loop of two lines, beside `CROSS`.
///
/// Reported as `icache6 one <cold> <warm> ..`, then
/// `costs loop1-<kind>` and `costs loop2-lw`, hundredths of a cycle a
/// round.
const ONE: usize = map::DDR3 + 0x3d_2000;
const ONE_LW: usize = map::DDR3 + 0x3e_2000;
const ONE_SW: usize = map::DDR3 + 0x3f_2000;
const ONE_COPY: usize = map::DDR3 + 0x2e_2000;
const TWO_LW: usize = map::DDR3 + 0x2f_2000;
const _: () = assert!((ONE - map::DDR3) % 0x4000 == 0x2000);
const _: () = assert!((ONE_LW - map::DDR3) % 0x4000 == 0x2000);
const _: () = assert!((ONE_SW - map::DDR3) % 0x4000 == 0x2000);
const _: () = assert!((ONE_COPY - map::DDR3) % 0x4000 == 0x2000);
const _: () = assert!((TWO_LW - map::DDR3) % 0x4000 == 0x2000);
// The two below 0x30_2000 sit past `STRETCH`'s 64 KiB and before the
// first `icache` block, a line of four words and a loop of four lines
// each.
const _: () = assert!(ONE_COPY >= STRETCH + 0x1_0000);
const _: () = assert!(TWO_LW + 64 <= map::DDR3 + 0x30_2000);

/// A loop of one line at `at`: the header, then `a`, `b`, `c`, `d`, the
/// counter and the branch in the second line's first three words, the
/// branch's target its first word, so no round crosses a line. The
/// branch's offset is `loop2`'s, -10.
fn loop1(at: usize, [a, b, c, d]: [u32; 4]) {
    put(at, [CSRR_T0, LI_A1, LUI_A2, LUI_A5]);
    put(
        at + 16,
        [pair(a, b), pair(c, d), pair(C_DEC_A1, C_BNEZ_A1), CSRR_T1],
    );
    put(at + 32, [SUB_A0, RET, NOP, NOP]);
}

/// The five loops that separate the fetch's stalls from the data's
/// (#1918).
fn icache6() {
    loop1(ONE, [C_NOP, C_NOP, C_NOP, C_NOP]);
    loop1(ONE_LW, [C_LW, C_NOP, C_NOP, C_NOP]);
    loop1(ONE_SW, [C_NOP, C_NOP, C_SW, C_NOP]);
    loop1(ONE_COPY, [C_LW, C_ADD_A2, C_SW, C_ADD_A5]);
    loop2(TWO_LW, [C_LW, C_NOP, C_NOP, C_NOP]);
    fence_i();
    let blocks = [ONE, ONE_LW, ONE_SW, ONE_COPY, TWO_LW];
    let rets = [
        ONE + 36,
        ONE_LW + 36,
        ONE_SW + 36,
        ONE_COPY + 36,
        TWO_LW + 48,
    ];
    for ret in rets {
        if unsafe { core::ptr::read_volatile(ret as *const u32) != RET } {
            Uart::say(b"costs icache6 blocks not written\n");
            return;
        }
    }
    let names = [&b"one"[..], b"one-lw", b"one-sw", b"one-copy", b"two-lw"];
    let keys = [
        &b"loop1-nop"[..],
        b"loop1-lw",
        b"loop1-sw",
        b"loop1-copy",
        b"loop2-lw",
    ];
    let mut cold = [0u32; 5];
    let mut warm = [0u32; 5];
    for (i, at) in blocks.into_iter().enumerate() {
        fence_i();
        cold[i] = run(at);
        warm[i] = run(at);
    }
    Uart::say(b"icache6");
    for (i, name) in names.iter().enumerate() {
        Uart::put(b' ');
        Uart::say(name);
        Uart::put(b' ');
        Uart::put_decimal(cold[i]);
        Uart::put(b' ');
        Uart::put_decimal(warm[i]);
    }
    Uart::put(b'\n');
    for (i, key) in keys.iter().enumerate() {
        Uart::say(b"costs ");
        Uart::say(key);
        Uart::put(b' ');
        Uart::put_decimal(warm[i] * 100 / 64);
        Uart::put(b'\n');
    }
}

/// The bulk blocks' two megabytes: a read and a set walk `FROM`, and a
/// copy moves it to `TO`, a word at a time, four words a round, as the
/// kernel's `memset` and `memcpy` do (issue 1561).
const FROM: usize = map::DDR3 + 0x40_0000;
const TO: usize = map::DDR3 + 0x60_0000;
const BULK: usize = 0x10_0000;

/// The cycles of one pass of `$body`, four words a round from `a0` up
/// to `a1`, with `a2` the copy's target, and a fence after.
macro_rules! bulk {
    ($($body:literal),*) => {{
        let t0 = mcycle();
        unsafe {
            asm!(
                "1:",
                $($body,)*
                "addi a0, a0, 16",
                "bne a0, a1, 1b",
                "fence",
                inout("a0") FROM => _,
                in("a1") FROM + BULK,
                inout("a2") TO => _,
                out("t0") _,
                out("t1") _,
                out("t2") _,
                out("t3") _,
            )
        };
        mcycle().wrapping_sub(t0)
    }};
}

/// A megabyte read, set and copied, each from memory the data cache
/// does not hold: `<tag> read <c> set <c> copy <c>`, the cycles of each,
/// then `costs <tag>-<kind> <hundredths of a cycle a word>`.
fn bulk(tag: &[u8]) {
    let read = bulk!(
        "lw t0, 0(a0)",
        "lw t1, 4(a0)",
        "lw t2, 8(a0)",
        "lw t3, 12(a0)"
    );
    let set = bulk!(
        "sw x0, 0(a0)",
        "sw x0, 4(a0)",
        "sw x0, 8(a0)",
        "sw x0, 12(a0)"
    );
    let copy = bulk!(
        "lw t0, 0(a0)",
        "lw t1, 4(a0)",
        "lw t2, 8(a0)",
        "lw t3, 12(a0)",
        "sw t0, 0(a2)",
        "sw t1, 4(a2)",
        "sw t2, 8(a2)",
        "sw t3, 12(a2)",
        "addi a2, a2, 16"
    );
    Uart::say(tag);
    Uart::say(b" read ");
    Uart::put_decimal(read);
    Uart::say(b" set ");
    Uart::put_decimal(set);
    Uart::say(b" copy ");
    Uart::put_decimal(copy);
    Uart::put(b'\n');
    let words = (BULK / 4) as u64;
    for (name, c) in [(&b"read"[..], read), (b"set", set), (b"copy", copy)] {
        Uart::say(b"costs ");
        Uart::say(tag);
        Uart::put(b'-');
        Uart::say(name);
        Uart::put(b' ');
        Uart::put_decimal((c as u64 * 100 / words) as u32);
        Uart::put(b'\n');
    }
}

/// Where the scanned pass shows from: a frame of the DDR3 clear of the
/// blocks', which the scanout reads whatever it holds.
const FRAME: u32 = (map::DDR3 + 0x200_0000) as u32;

/// The bulk blocks again with the scanout showing a frame from the DDR3,
/// as Linux's boot on the board has it from the shim on (issue 1561):
/// `costs scan-bulk-<kind>`. The scanout reads 640 by 480 words sixty
/// times a second from the same controller, which the model does not
/// charge, so the model gives the same figures as `bulk`'s and the board
/// says what the scanout costs a stream. The scanout takes the frame at
/// the next vertical blanking, so the pass waits two frames first.
fn scanned() {
    Scan::base(FRAME);
    Scan::show(true);
    let t0 = mcycle();
    while mcycle().wrapping_sub(t0) < 2 * 1_668_321 {}
    bulk(b"scan-bulk");
    scan_icache();
    Scan::show(false);
}

/// `icache`'s miss again with the scanout showing (issue 1561): `A` run
/// cold and then warm, `scan-icache cold <c> warm <w>`, and `costs
/// scan-icache-miss <hundredths of a cycle a line>`, beside the same
/// run's `icache-miss`, which `icache` timed with the scanout off. Every
/// figure the instruction cache is charged was timed that way, while
/// Linux's boot fetches with the scanout showing from the shim on.
fn scan_icache() {
    if !written(A) {
        Uart::say(b"costs scan-icache block not written\n");
        return;
    }
    fence_i();
    let cold = run(A) as i32;
    let warm = run(A) as i32;
    Uart::say(b"scan-icache cold ");
    Uart::put_decimal(cold as u32);
    Uart::say(b" warm ");
    Uart::put_decimal(warm as u32);
    Uart::put(b'\n');
    Uart::say(b"costs scan-icache-miss");
    put_line(cold - warm, 1);
}

/// The page walk's blocks: an Sv32 root table at `PT` and a second-level
/// table after it, mapping the 4 KiB page at `VA_PAGE` and the 4 MiB
/// megapage at `VA_MEGA` both onto `WORD`'s.
const PT: usize = map::DDR3 + 0x70_0000;
const VA_PAGE: usize = 0x0040_0000;
const VA_MEGA: usize = 0x0080_0000;
/// Valid, readable, writable, accessed and dirty.
const LEAF: u32 = 0xc7;

/// `mstatus`'s `MPRV`, with `MPP` supervisor: a load in machine mode is
/// translated as the supervisor's, while fetches are not.
const MPRV_S: usize = (1 << 17) | (1 << 11);

/// The cycles of `N` rounds of `$body` with `MPRV` set, loads at `a3`;
/// `MPRV` is cleared again inside the same block, so nothing the
/// compiler writes runs translated.
macro_rules! walked {
    ($at:expr, $($body:literal),*) => {{
        let c: u32;
        unsafe {
            asm!(
                "csrc mstatus, {mpp}",
                "csrs mstatus, {mprv}",
                "csrr {t0}, mcycle",
                "1:",
                $($body,)*
                "addi {n}, {n}, -1",
                "bnez {n}, 1b",
                "csrr {c}, mcycle",
                "csrc mstatus, {mprv}",
                "sub {c}, {c}, {t0}",
                mpp = in(reg) 3usize << 11,
                mprv = in(reg) MPRV_S,
                t0 = out(reg) _,
                c = out(reg) c,
                n = inout(reg) N => _,
                in("a3") $at,
                out("a2") _,
            )
        };
        c
    }};
}

/// A data walk after `sfence.vma`, through a page and through a
/// megapage, with the tables in a DDR3 row the walks keep open: `walk
/// page <c> mega <c> sfence <c> hit <c> empty <c>`, the cycles of `N`
/// rounds of each, then `costs walk-page` and `costs walk-mega`, a
/// walk's cycles in hundredths: the round less `sfence.vma` alone and a
/// load the TLB holds, with the loop counted once.
fn walk() {
    let root = PT as *mut u32;
    let second = (PT + 0x1000) as *mut u32;
    let ppn = |at: usize| (at >> 12) as u32;
    unsafe {
        core::ptr::write_volatile(
            root.add(VA_PAGE >> 22),
            ppn(PT + 0x1000) << 10 | 1,
        );
        core::ptr::write_volatile(
            root.add(VA_MEGA >> 22),
            ppn(map::DDR3) << 10 | LEAF,
        );
        core::ptr::write_volatile(
            second.add((VA_PAGE >> 12) & 0x3ff),
            ppn(WORD) << 10 | LEAF,
        );
        asm!("fence", "csrw satp, {0}", "sfence.vma", in(reg) (1 << 31) | ppn(PT));
    }
    let page = VA_PAGE + (WORD & 0xfff);
    let mega = VA_MEGA + (WORD - map::DDR3);
    let p = walked!(page, "sfence.vma", "lw a2, 0(a3)");
    let m = walked!(mega, "sfence.vma", "lw a2, 0(a3)");
    let s = walked!(page, "sfence.vma");
    let h = walked!(page, "lw a2, 0(a3)");
    let e = walked!(page,);
    unsafe { asm!("csrw satp, x0", "sfence.vma") };
    Uart::say(b"walk page ");
    for (i, c) in [p, m, s, h, e].into_iter().enumerate() {
        if i > 0 {
            Uart::say(
                [&b" mega "[..], b" sfence ", b" hit ", b" empty "][i - 1],
            );
        }
        Uart::put_decimal(c);
    }
    Uart::put(b'\n');
    for (name, c) in [(&b"page"[..], p), (b"mega", m)] {
        let per = (c as i32 - s as i32 - h as i32 + e as i32) * 100 / N as i32;
        Uart::say(b"costs walk-");
        Uart::say(name);
        Uart::put(b' ');
        if per < 0 {
            Uart::put(b'-');
        }
        Uart::put_decimal(per.unsigned_abs());
        Uart::put(b'\n');
    }
}

/// One of the translation blocks' loops, 32 rounds of `$body` twice and
/// the second kept, timed with `rdcycle`, its cycles stored at `$off`
/// from `a1`. A round's far side, for the jumps, is placed after the
/// loop, past `$gap` bytes, and jumps back to the round's end.
macro_rules! xloop {
    ($body:literal, $gap:literal, $off:literal) => {
        concat!(
            "li t1, 2\n",
            "30: li t0, 32\n",
            "rdcycle t2\n",
            "31: ",
            $body,
            "\n",
            "32: addi t0, t0, -1\n",
            "bnez t0, 31b\n",
            "rdcycle t3\n",
            "addi t1, t1, -1\n",
            "bnez t1, 30b\n",
            "sub t3, t3, t2\n",
            "sw t3, ",
            $off,
            "(a1)\n",
            "j 34f\n",
            ".skip ",
            $gap,
            "\n",
            "33: j 32b\n",
            "34:\n",
        )
    };
}

/// The five loops: a load, the empty loop, a store, a jump to a label in
/// the same page, and one to a label on the next page.
macro_rules! xloops {
    ($a:literal, $b:literal, $c:literal, $d:literal, $e:literal) => {
        concat!(
            xloop!("lw t4, 0(a0)", "0", $a),
            xloop!("addi t4, t4, 1", "0", $b),
            xloop!("sw t4, 0(a0)", "0", $c),
            xloop!("j 33f", "0", $d),
            xloop!("j 33f", "4096", $e),
        )
    };
}

/// The cycles of the translation blocks: supervisor mode's five, then
/// machine mode's.
static mut XLATE: [u32; 10] = [0; 10];

/// What translation costs the core (issue 1561): the same five loops in
/// supervisor mode, with this program's own 4 MiB of the DDR3 mapped to
/// itself by a megapage, and in machine mode, untranslated. Under
/// translation the core waits for the unit's answer on each load and
/// store, and each time the fetch moves to another page, which machine
/// mode never does; the model charges that as `Timing::xlate`, 4 from
/// the RTL. `smode s <load> <alu> <store> <near> <far> m <...>`, the
/// cycles of 32 rounds each, then `costs xlate-load`, `xlate-store` and
/// `xlate-move` in hundredths of a cycle: a load's or a store's round
/// beyond the empty loop's, and half the far jump's beyond the near
/// one's, each supervisor mode's less machine mode's.
fn smode() {
    let root = (PT + 0x2000) as *mut u32;
    let ppn = |at: usize| (at >> 12) as u32;
    unsafe {
        for i in 0..1024 {
            core::ptr::write_volatile(root.add(i), 0);
        }
        // Valid, readable, writable, runnable, accessed and dirty.
        core::ptr::write_volatile(
            root.add(map::DDR3 >> 22),
            ppn(map::DDR3) << 10 | LEAF | 8,
        );
        asm!(
            "fence",
            "csrw satp, {0}",
            "sfence.vma",
            in(reg) (1 << 31) | ppn(PT + 0x2000)
        );
    }
    let out = core::ptr::addr_of_mut!(XLATE) as usize;
    unsafe {
        asm!(
            // No interrupt while out of machine mode, and the trap that
            // brings it back to the label after supervisor mode's loops.
            "csrrw {mie}, mie, x0",
            "la t0, 5f",
            "csrrw {mtvec}, mtvec, t0",
            "csrwi mcounteren, 1",
            "li t0, 0x1800",
            "csrc mstatus, t0",
            "li t0, 0x800",
            "csrs mstatus, t0",
            "la t0, 2f",
            "csrw mepc, t0",
            "mret",
            // Each mode's copy of the loops starts a page, so that each
            // round's lines lie the same for both.
            ".balign 4096",
            "2:",
            xloops!("0", "4", "8", "12", "16"),
            "ecall",
            ".balign 4096",
            "5:",
            xloops!("20", "24", "28", "32", "36"),
            "csrw mtvec, {mtvec}",
            "csrw mie, {mie}",
            "csrw satp, x0",
            "sfence.vma",
            mie = out(reg) _,
            mtvec = out(reg) _,
            in("a0") WORD,
            in("a1") out,
            out("t0") _,
            out("t1") _,
            out("t2") _,
            out("t3") _,
            out("t4") _,
        );
    }
    let c: [u32; 10] =
        unsafe { core::ptr::read_volatile(core::ptr::addr_of!(XLATE)) };
    Uart::say(b"smode s");
    for (i, v) in c.iter().enumerate() {
        if i == 5 {
            Uart::say(b" m");
        }
        Uart::put(b' ');
        Uart::put_decimal(*v);
    }
    Uart::put(b'\n');
    let d = |k: usize| c[k] as i32 - c[5 + k] as i32;
    let kinds = [
        (&b"load"[..], (d(0) - d(1)) * 100 / 32),
        (b"store", (d(2) - d(1)) * 100 / 32),
        (b"move", (d(4) - d(3)) * 100 / 64),
    ];
    for (name, per) in kinds {
        Uart::say(b"costs xlate-");
        Uart::say(name);
        Uart::put(b' ');
        if per < 0 {
            Uart::put(b'-');
        }
        Uart::put_decimal(per.unsigned_abs());
        Uart::put(b'\n');
    }
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
    icache_guess_contend();
    icache4();
    icache5();
    icache6();
    // A load from the data memory at 0x1000, `DATA_BASE`, over the bus
    // (#1755): the data cache holds only the DDR3's lines, so the load
    // goes around it every time. The `lui` is a cycle the row carries on
    // every board alike, so two boards compare.
    say(b"dmem-load", timed!("lui a3, 1", "lw a2, 0(a3)"), empty);
    bulk(b"bulk");
    walk();
    smode();
    scanned();
    Uart::say(b"costs done\n");
    halt()
}
