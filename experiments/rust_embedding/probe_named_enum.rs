// SPDX-License-Identifier: Apache-2.0
//! Probe: `#[named_type_params]` on an enum (#1899). Expected to fail,
//! with the macro's own message:
//!
//! ```text
//! error: `#[named_type_params]` goes on a struct or a type alias, not
//!        an enum or a union
//! ```
use txhdl::named_type_params;

#[named_type_params]
pub enum Either<Left, Right> {
    L(Left),
    R(Right),
}
