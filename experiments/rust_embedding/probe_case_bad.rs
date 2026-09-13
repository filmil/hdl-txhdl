// SPDX-License-Identifier: Apache-2.0
// Probe 10d. A drive with no arrow inside a `case!` arm. Expected to
// fail with the macro's own message, the same one `when!` gives.
use txhdl::case;
use txhdl::comp::Reg;
use txhdl::types::U;

#[derive(Copy, Clone, Default, PartialEq)]
pub enum Op { #[default] Nop, Inc }

pub struct Unit { pub acc: Reg<U<8>> }

impl Unit {
    pub fn step(&self, op: Op, acc: U<8>) {
        case!(op => {
            Op::Inc => { self.acc acc },   // no `<=`
            _ => {},
        });
    }
}
