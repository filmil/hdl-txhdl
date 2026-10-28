// SPDX-License-Identifier: Apache-2.0
//! Probe: `#[named_type_params(path = ..)]` naming a module the struct
//! is not in (#1915). Expected to fail at the struct, not at a use:
//!
//! ```text
//! error[E0432]: unresolved import `crate`
//!   #[named_type_params(path = crate::elsewhere)]
//!   ^^^ could not find `elsewhere` in the crate root
//! ```
//!
//! The attribute checks its path with `use crate::m::Name as _;` beside
//! the struct.
pub mod here {
    use txhdl::named_type_params;

    #[named_type_params(path = crate::elsewhere)]
    pub struct Lanes<const WIDTH: usize> {
        pub lanes: [u8; WIDTH],
    }
}
