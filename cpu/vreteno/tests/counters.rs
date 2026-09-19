// SPDX-License-Identifier: Apache-2.0
//! The two machine counters, checked by a program that measures
//! itself.
//!
//! This is not a lockstep test, and it cannot be one. The model steps
//! when the core retires, so it knows how many instructions have gone
//! by and has no idea how many cycles the pipeline spent; a program
//! that read `mcycle` into a register would get a number from the core
//! that no model could produce without knowing the microarchitecture,
//! which is the thing the model exists not to know. So the counters
//! are checked here, against the core alone, on the properties a
//! program can rely on.
use vreteno32::program::measure;
use vreteno32::run::run;

#[test]
fn a_program_measures_itself_with_the_counters() {
    let ran = run(&measure(), &[], 4000);
    assert!(ran.halted_at.is_some(), "the program did not halt");
    let (cycles, retired, quotient) = (ran.mem[0], ran.mem[1], ran.mem[2]);

    // The division is the point of the program: it retires one
    // instruction and takes about thirty-three cycles, so the two
    // counters cannot agree.
    assert_eq!(quotient, 1000 / 7, "the division, so the work happened");
    assert_eq!(retired, 5, "instructions between the reads");
    assert!(
        cycles > retired + 20,
        "cycles {cycles} against instructions {retired}: a division \
         should leave a gap of about thirty, and a core whose `mcycle` \
         counted retirements rather than cycles would show none"
    );
    // A sanity bound on the other side: the stretch is a handful of
    // instructions and one division, not a thousand cycles.
    assert!(cycles < 200, "cycles between the reads: {cycles}");
}

#[test]
fn the_counters_read_rather_than_trapping() {
    // Before issue 299 every one of these was an illegal instruction.
    // The program above reads two of them; this says the run reached
    // its halt at all, which it could not have done if a read had
    // trapped into a handler that was never installed.
    let ran = run(&measure(), &[], 4000);
    assert!(
        ran.halted_at.is_some(),
        "a read of a counter trapped instead of answering"
    );
}
