// SPDX-License-Identifier: Apache-2.0
//! Probe: a const parameter given by name the way `#[named_type_params]`
//! gives a type parameter (#1899), as a marker type with the number in
//! an associated constant, read where the struct needs the number.
//! Expected to fail on stable Rust: a generic parameter may not be used
//! in a const expression, so the projection cannot reach an array's
//! length or a `U<N>`:
//!
//! ```text
//! error: generic parameters may not be used in const operations
//!   --> probe_named_const.rs:23:19
//!    |     pub data: U<{ W::VALUE }>,
//! error: generic parameters may not be used in const operations
//!   --> probe_named_const.rs:24:21
//!    |     pub lanes: [u8; W::VALUE],
//! ```
use txhdl::types::U;

pub struct Width<const V: usize>;
pub trait IsWidth {
    const VALUE: usize;
}
impl<const V: usize> IsWidth for Width<V> {
    const VALUE: usize = V;
}

pub struct Bus<W: IsWidth> {
    pub data: U<{ W::VALUE }>,
    pub lanes: [u8; W::VALUE],
}
