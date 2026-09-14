// SPDX-License-Identifier: Apache-2.0
// Probe 10e. A binding pattern in a `case!` arm. Expected to fail: the
// macro tests each pattern with `matches!`, whose bindings are scoped to
// itself, so `v` is not in scope in the arm's body. A payload is read
// from the value by a function instead.
use txhdl::case;
use txhdl::comp::Reg;
use txhdl::types::U;

#[derive(Copy, Clone, Default, PartialEq)]
pub enum Op {
    #[default]
    Nop,
    Load(u8),
}

pub struct Unit {
    pub acc: Reg<U<8>>,
}

impl Unit {
    pub fn step(&self, op: Op) {
        case!(op => {
            Op::Load(v) => { self.acc <= U::from(v) },
            _ => {},
        });
    }
}
