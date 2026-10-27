// SPDX-License-Identifier: Apache-2.0
//! Probe: `#[named_type_params]` on a type alias with a type parameter
//! (#1946). An alias makes no markers, so it names const parameters
//! only. Expected to fail, with the macro's own message:
//!
//! ```text
//! error: on a type alias, `#[named_type_params]` names const
//!        parameters only: an alias has no markers to name a type by
//! ```
use txhdl::named_type_params;

pub struct Pair<A, B>(pub A, pub B);

#[named_type_params]
pub type Same<T> = Pair<T, T>;
