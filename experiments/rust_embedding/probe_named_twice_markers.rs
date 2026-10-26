// SPDX-License-Identifier: Apache-2.0
//! Probe: two structs in one module that each make the marker `Data`
//! (#1914). Expected to fail, which is why `reuse(..)` exists:
//!
//! ```text
//! error[E0428]: the name `Data` is defined multiple times
//! error[E0428]: the name `IsData` is defined multiple times
//! ```
use txhdl::named_type_params;

#[named_type_params]
pub struct Wire<Data: Copy> {
    pub value: Data,
}

#[named_type_params]
pub struct Port<Data: Copy> {
    pub value: Data,
}
