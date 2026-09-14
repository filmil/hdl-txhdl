// SPDX-License-Identifier: Apache-2.0
// Probe 10b. An entry with no colon inside `when!`. Expected to fail
// with the macro's own message rather than a type error after
// expansion.
use txhdl::comp::Reg;
use txhdl::types::{Bit, U};
use txhdl::when;

pub struct Unit {
    pub count: Reg<U<32>>,
}

impl Unit {
    pub fn step(&self, enable: Bit) {
        when!(enable => self {
            count 0    // no `:`
        });
    }
}
