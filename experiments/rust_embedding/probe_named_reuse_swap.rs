// SPDX-License-Identifier: Apache-2.0
//! Probe: two shared markers swapped at a use (#1914). Expected to
//! fail: reuse keeps the guarantee, since `Domain<..>` is not `IsData`:
//!
//! ```text
//! error[E0277]: the trait bound `Domain<DefaultClock>: IsData` is
//!        not satisfied
//! error[E0277]: the trait bound `Data<Bit>: IsDomain` is not
//!        satisfied
//! ```
use txhdl::comp::{Data, DefaultClock, Domain, Signal};
use txhdl::types::Bit;

pub fn take(_s: Signal<Domain<DefaultClock>, Data<Bit>>) {}
