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
use vreteno32::dmem::Dmem;

/// A data memory of twelve address bits and the default 1024 words.
pub fn mismatched() -> Dmem<2, 12> {
    Dmem::default()
}
