// SPDX-License-Identifier: Apache-2.0
//! Run the traps program on the core, and check that its handlers ran.
//!
//! The program in `cpu/vreteno/rust/traps.rs` makes three environment
//! calls and takes one timer interrupt, all through the dispatcher in
//! `vreteno_hal::trap`, and says what it counted. The machine it runs
//! on is `vreteno32::run`, as for the greeting.
use vreteno32::run::{expect, run};

/// What the program is written to print.
const EXPECTED: &str = "ecall 3 timer 1\n";

fn main() {
    let ran = run(traps_program::TEXT, traps_program::DATA, 8000);
    expect("the traps program", &ran, EXPECTED);
    println!("Three environment calls and a timer interrupt, dispatched.");
}

/// The same run, as a test.
#[cfg(test)]
mod tests {
    use super::EXPECTED;
    use vreteno32::run::{expect, run};

    #[test]
    fn traps_dispatch_through_the_crate() {
        let ran = run(traps_program::TEXT, traps_program::DATA, 8000);
        expect("the traps program", &ran, EXPECTED);
    }
}
