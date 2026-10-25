// SPDX-License-Identifier: Apache-2.0
//! Probe: `Signal`'s two named parameters swapped (#1899). Expected to
//! fail: `Domain<DefaultClock>` is not `IsData`, which `Signal`'s first
//! parameter needs, so the swap is refused where it is written:
//!
//! ```text
//! error[E0277]: the trait bound `Domain<DefaultClock>: IsData` is
//!        not satisfied
//! error[E0277]: the trait bound `Data<Bit>: IsDomain` is not
//!        satisfied
//! ```
use txhdl::comp::{Data, DefaultClock, Domain, Signal};
use txhdl::types::Bit;

pub type Swapped = Signal<Domain<DefaultClock>, Data<Bit>>;
pub fn take(_s: Swapped) {}
