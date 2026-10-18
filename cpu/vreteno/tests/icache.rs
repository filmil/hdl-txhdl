// SPDX-License-Identifier: Apache-2.0
//! The instruction cache (issue 1021): a program fetched from the data
//! memory over the bus runs at close to the speed of one in the boot
//! memory, and code written by stores is what runs after a `fence.i`.
//! Its counters (#1752) count lookups, hits, fills started, fills
//! kept and fills dropped.
//!
//! The loop is issue 1009's measurement (docs/sv32-timing.md): three
//! instructions, a thousand times round. Before the cache it ran at
//! 1.333 cycles an instruction from the boot memory and at 14.655 from
//! the data memory, a word over the bus for every instruction.
use vreteno32::isa::{
    addi, bne, csrrs, csrrw, fence_i, halt, jalr, lui, sw, CSR_IC_DROPPED,
    CSR_IC_FILLS, CSR_IC_HITS, CSR_IC_KEPT, CSR_IC_LOOKUPS,
};
use vreteno32::run::run;

/// The loop, which halts after a thousand rounds.
fn body() -> Vec<u32> {
    vec![
        addi(2, 0, 1000),
        addi(1, 1, 1),
        addi(2, 2, -1),
        bne(2, 0, -8),
        halt(),
    ]
}

fn bytes(words: &[u32]) -> Vec<u8> {
    words.iter().flat_map(|w| w.to_le_bytes()).collect()
}

/// Cycles per instruction: the cycles to the halt over the
/// instructions retired, which for the loop is its three a round and
/// the four around it.
fn cpi(halted_at: Option<u64>, instrs: u64) -> f64 {
    halted_at.expect("the loop halts") as f64 / instrs as f64
}

#[test]
fn a_loop_from_the_data_memory_runs_from_the_cache() {
    let instrs = 3 * 1000 + 2;
    let near = run(&body(), &[], 200_000);
    // From the data memory at 0x1000: the boot memory jumps there.
    let far = run(&[lui(5, 1), jalr(0, 5, 0)], &bytes(&body()), 200_000);
    let (n, f) = (cpi(near.halted_at, instrs), cpi(far.halted_at, instrs));
    println!("loop, boot memory: CPI {n:.3}");
    println!("loop, data memory: CPI {f:.3} (14.655 before the cache)");
    // The first round fills the loop's line; every round after it
    // hits. The bound is loose on purpose: what it rules out is a word
    // over the bus for every instruction.
    assert!(f < 4.5, "the loop from the data memory ran at {f:.3}");
}

/// A routine at 0x1100 that returns 1 in x10, rewritten by a store to
/// return 2, and called again after a `fence.i`: the second call runs
/// the stored word, not the line the first call left in the cache.
#[test]
fn code_stored_is_what_runs_after_fence_i() {
    // The new first word, `addi x10, x0, 2`, built in x7.
    let new = addi(10, 0, 2);
    let (hi, lo) = ((new.wrapping_add(0x800)) >> 12, (new & 0xfff) as i32);
    let lo = if lo >= 0x800 { lo - 0x1000 } else { lo };
    let text = [
        lui(5, 1),         // x5 = 0x1000, the data memory
        addi(6, 5, 0x100), // x6 = 0x1100, the routine
        jalr(1, 6, 0),     // call it: x10 = 1
        sw(10, 5, 0),      // the first answer, at 0x1000
        lui(7, hi),
        addi(7, 7, lo), // x7 = the new first word
        sw(7, 6, 0),    // rewrite the routine
        fence_i(),
        jalr(1, 6, 0), // call it again: x10 = 2
        sw(10, 5, 4),  // the second answer, at 0x1004
        halt(),
    ];
    let mut data = vec![0u8; 0x100];
    data.extend(bytes(&[addi(10, 0, 1), jalr(0, 1, 0)]));
    let ran = run(&text, &data, 20_000);
    assert!(ran.halted_at.is_some(), "the program halts");
    assert_eq!(ran.mem[0], 1, "the first call ran the routine as loaded");
    assert_eq!(ran.mem[1], 2, "the second ran the word stored");
}

/// A loop that crosses from one line into the next, so both lines are
/// filled and the fetch goes from one to the other every round.
#[test]
fn a_loop_across_two_lines_runs_from_the_cache() {
    // Two words of padding put the loop's branch in the second line.
    let mut words = vec![addi(0, 0, 0), addi(0, 0, 0)];
    words.extend(body());
    let far = run(&[lui(5, 1), jalr(0, 5, 0)], &bytes(&words), 200_000);
    let f = cpi(far.halted_at, 3 * 1000 + 4);
    println!("loop across two lines, data memory: CPI {f:.3}");
    assert!(f < 4.5, "the loop across two lines ran at {f:.3}");
}

/// The five counters, low halves then high, in the order of their
/// addresses: lookups, hits, fills started, fills kept, fills dropped.
const COUNTERS: [u32; 5] = [
    CSR_IC_LOOKUPS,
    CSR_IC_HITS,
    CSR_IC_FILLS,
    CSR_IC_KEPT,
    CSR_IC_DROPPED,
];

/// The loop from the data memory at 0x1100, called from the boot
/// memory, which then stores the cache's five counters at 0x1000 and
/// their upper halves after them (#1752). The boot memory is not
/// cached, so the counters stand still while they are read, and a
/// hit is a lookup that hit at the first look (#1781): lookups less
/// hits are the fills started, every one of them kept or dropped.
#[test]
fn the_counters_count_the_loops_lookups_hits_and_fills() {
    let mut code = body();
    code.pop(); // the halt
    code.push(jalr(0, 3, 0)); // back to the boot memory
    let mut boot = vec![lui(5, 1), addi(5, 5, 0x100), jalr(3, 5, 0)];
    // A few cycles for a fill still out to end.
    boot.extend([addi(0, 0, 0); 8]);
    boot.push(lui(6, 1)); // x6 = 0x1000, where the counts go
    for (i, csr) in COUNTERS.iter().enumerate() {
        boot.push(csrrs(10, *csr, 0));
        boot.push(sw(10, 6, 4 * i as i32));
        boot.push(csrrs(10, csr + 0x80, 0));
        boot.push(sw(10, 6, 20 + 4 * i as i32));
    }
    boot.push(halt());
    let mut data = vec![0u8; 0x100];
    data.extend(bytes(&code));
    let ran = run(&boot, &data, 200_000);
    assert!(ran.halted_at.is_some(), "the program halts");
    let [lookups, hits, fills, kept, dropped] =
        [0, 1, 2, 3, 4].map(|i| ran.mem[i]);
    println!(
        "lookups {lookups}, hits {hits}, fills {fills}, kept {kept}, \
         dropped {dropped}"
    );
    assert_eq!(&ran.mem[5..10], &[0; 5], "the upper halves");
    // Three instructions a round, and every round after the first
    // fetches its three from the line the first one filled.
    assert!(hits >= 3 * 999, "hits {hits}");
    // Every miss started a fill, and nothing else did (#1781).
    assert_eq!(lookups - hits, fills, "{lookups} lookups, {hits} hits");
    assert_eq!(kept + dropped, fills, "{kept} kept, {dropped} dropped");
    // The code is a line or two long, each filled once: what this rules
    // out is a fill for every fetch.
    assert!((1..=4).contains(&kept), "fills kept {kept}");
}
/// A program that never leaves the boot memory, which is not cached,
/// counts nothing, so each of the ten halves reads back what was
/// written to it, and machine mode reads the low halves again as
/// `hpmcounter3` to `7`.
#[test]
fn the_counters_read_what_was_written_and_count_no_boot_fetch() {
    let mut text = vec![lui(6, 1)]; // x6 = 0x1000, where the counts go
    let halves: Vec<u32> = COUNTERS
        .iter()
        .copied()
        .chain(COUNTERS.iter().map(|c| c + 0x80))
        .collect();
    for (i, csr) in halves.iter().enumerate() {
        text.push(addi(7, 0, 100 + i as i32));
        text.push(csrrw(0, *csr, 7));
    }
    for (i, csr) in halves.iter().enumerate() {
        text.push(csrrs(10, *csr, 0));
        text.push(sw(10, 6, 4 * i as i32));
    }
    for (i, csr) in COUNTERS.iter().enumerate() {
        text.push(csrrs(10, csr + 0x100, 0)); // 0xc03 up
        text.push(sw(10, 6, 40 + 4 * i as i32));
    }
    text.push(halt());
    let ran = run(&text, &[], 20_000);
    assert!(ran.halted_at.is_some(), "the program halts");
    let written: Vec<u32> = (100..110).collect();
    assert_eq!(&ran.mem[0..10], &written[..], "each half as written");
    assert_eq!(&ran.mem[10..15], &written[0..5], "the user's copies");
}
