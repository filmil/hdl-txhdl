// SPDX-License-Identifier: Apache-2.0
//! Expected to fail (#1971): a `Dmem` whose `WORDS` is not `1 << ADDR`.
//! `Dmem<2, 12>` asks for 4096 words' worth of addresses and leaves
//! `WORDS` at its default of 1024, which positional parameters allowed
//! and named ones make easier to write. `Dmem::SHAPE` refuses it when the
//! program is compiled:
//!
//! ```text
//! error[E0080]: evaluation panicked: Dmem: WORDS must be 1 << ADDR
//! ```
//!
//! The same for the other half given alone, by name: 4096 words with
//! `ADDR` left at 10.
use vreteno32::dmem::Dmem;
use vreteno32::Dmem;

/// A data memory of twelve address bits and the default 1024 words.
pub fn mismatched() -> Dmem<2, 12> {
    Dmem::default()
}

/// 4096 words, numbered by the default ten bits.
pub fn mismatched_by_name() -> Dmem![ID = 2, WORDS = 4096] {
    Dmem::default()
}
