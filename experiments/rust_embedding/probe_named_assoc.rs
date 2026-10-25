// SPDX-License-Identifier: Apache-2.0
//! Probe: `P::Name` in a field (#1899). Expected to fail, with the
//! macro's own message: a projection cannot be followed by `::`, so it
//! is written `<P as Trait>::Name`.
//!
//! ```text
//! error: `Src::..` cannot be read through `Src`'s marker: write
//!        `<Src as Trait>::..`
//! ```
use txhdl::named_type_params;

#[named_type_params]
pub struct Items<Src: Iterator> {
    pub first: Option<Src::Item>,
}
