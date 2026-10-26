// SPDX-License-Identifier: Apache-2.0
//! Probe: `reuse(..)` naming one marker twice (#1914). Expected to
//! fail, with the macro's own message:
//!
//! ```text
//! error: `reuse(..)` names `Data` twice
//! ```
use txhdl::{named_markers, named_type_params};

named_markers!(pub Data);

#[named_type_params(reuse(Data, Data))]
pub struct Wire<Data: Copy> {
    pub value: Data,
}
