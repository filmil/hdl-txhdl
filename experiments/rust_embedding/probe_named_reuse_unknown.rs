// SPDX-License-Identifier: Apache-2.0
//! Probe: `reuse(..)` naming no type parameter of the struct (#1914).
//! Expected to fail, with the macro's own message:
//!
//! ```text
//! error: `reuse(Value)` names no type parameter of `Wire`
//! ```
use txhdl::{named_markers, named_type_params};

named_markers!(pub Data);

#[named_type_params(reuse(Value))]
pub struct Wire<Data: Copy> {
    pub value: Data,
}
