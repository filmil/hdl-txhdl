// SPDX-License-Identifier: Apache-2.0
//! Probe: `Signal` given a bare type where its marker belongs (#1899).
//! Expected to fail: `Bit` is not `IsData`; the argument is
//! `Data<Bit>`:
//!
//! ```text
//! error[E0277]: the trait bound `txhdl::types::Bit:
//!        txhdl::comp::IsData` is not satisfied
//! ```
use txhdl::comp::Signal;
use txhdl::types::Bit;

pub fn take(_s: Signal<Bit>) {}
