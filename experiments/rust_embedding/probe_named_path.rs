// SPDX-License-Identifier: Apache-2.0
//! Probe: a struct's macro named by a path from inside its own crate
//! (#1915). Expected to fail: the macro is made by another macro,
//! `#[named_type_params]`, and Rust refuses such a macro by path in its
//! own crate:
//!
//! ```text
//! error: macro-expanded `macro_export` macros from the current crate
//!        cannot be referred to by absolute paths
//! ```
//!
//! So the use-site macro calls itself by its bare name, and
//! `#[named_type_params(path = ..)]` puts the path on the struct and the
//! markers only.
use txhdl::named_type_params;

#[named_type_params]
pub struct Lanes<const WIDTH: usize> {
    pub lanes: [u8; WIDTH],
}

pub type ByPath = crate::Lanes![WIDTH = 2];
