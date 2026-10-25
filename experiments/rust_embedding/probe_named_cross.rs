// SPDX-License-Identifier: Apache-2.0
//! Probe: a type parameter's bound that names another (#1899).
//! Expected to fail, with the macro's own message: a role is a trait
//! with no parameters, so the bound goes in the `where` clause.
//!
//! ```text
//! error: `Addr`'s bound names the parameter `Word`: put it in the
//!        `where` clause, `where Addr: ..`
//! ```
use txhdl::named_type_params;

#[named_type_params]
pub struct Bus<Word, Addr: Into<Word>> {
    pub addr: Addr,
    pub data: Word,
}
