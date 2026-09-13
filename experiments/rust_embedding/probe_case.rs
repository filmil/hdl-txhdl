// SPDX-License-Identifier: Apache-2.0
// Probe 10c. `case!`: `when!` with many arms. The arms are Rust patterns,
// guards, alternatives and `_` included, and the first match wins. A
// pattern tests; it does not bind (probe 10e).
use txhdl::case;
use txhdl::comp::Reg;
use txhdl::types::U;

#[derive(Copy, Clone, Default, PartialEq)]
pub enum Op { #[default] Nop, Inc, Dec, Load(u8) }

pub struct Unit { pub acc: Reg<U<8>>, pub op: Reg<Op> }

impl Unit {
    pub fn step(&self, op: Op, acc: U<8>) {
        case!(op => {
            Op::Inc => { self.acc <= acc.wrapping_add(1) },
            Op::Dec if acc == U::from(0) => { self.op <= Op::Nop },
            Op::Dec => { self.acc <= acc.wrapping_sub(1) },
            Op::Load(_) | Op::Nop => { self.op <= Op::Nop },
            _ => {},
        });
    }
}
