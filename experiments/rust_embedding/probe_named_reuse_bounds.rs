// SPDX-License-Identifier: Apache-2.0
//! Probe: a struct on shared markers still holds its own bounds
//! (#1914). Expected to fail: `Signal`'s data must be `Copy + Default`,
//! which its `where` clause states through the bound-free role, and a
//! `String` is not `Copy`:
//!
//! ```text
//! error[E0277]: the trait bound `String: Copy` is not satisfied
//! ```
use txhdl::comp::{Data, Signal};

pub fn take(_s: Signal<Data<String>>) {}
