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
//! So `#[named_type_params(path = ..)]` makes the macro under a hidden
//! name and re-exports it beside the struct, which is a path Rust does
//! let reach it; the macro calls itself through that re-export (#1915).
use txhdl::named_type_params;

#[named_type_params]
pub struct Lanes<const WIDTH: usize> {
    pub lanes: [u8; WIDTH],
}

pub type ByPath = crate::Lanes![WIDTH = 2];
