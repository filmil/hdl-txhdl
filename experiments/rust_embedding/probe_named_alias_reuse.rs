// SPDX-License-Identifier: Apache-2.0
//! Probe: `#[named_type_params(reuse(..))]` on a type alias (#1946).
//! `reuse` names shared markers, and an alias takes none. Expected to
//! fail, with the macro's own message:
//!
//! ```text
//! error: `reuse(..)` names markers, which a type alias does not take
//! ```
use txhdl::named_type_params;

pub struct Lanes<const WIDTH: usize, const COUNT: usize>(
    pub [[u8; WIDTH]; COUNT],
);

#[named_type_params(reuse(Data))]
pub type Row<const COUNT: usize, const WIDTH: usize> = Lanes<WIDTH, COUNT>;
