// SPDX-License-Identifier: Apache-2.0
//! Probe: a struct's macro given one parameter twice and another not at
//! all (#1899). Expected to fail, with the macro's own message:
//!
//! ```text
//! error: `Lanes![..]` gives each of its parameters by name, once:
//!        WIDTH, COUNT
//! ```
use txhdl::named_type_params;

#[named_type_params]
pub struct Lanes<const WIDTH: usize, const COUNT: usize> {
    pub lanes: [[u8; WIDTH]; COUNT],
}

pub type Twice = Lanes![WIDTH = 2, WIDTH = 3];
