// SPDX-License-Identifier: Apache-2.0
// Probe 15. The value types, from the library: Bit, Logic, U, I and
// logic::Vec, with resolution and X propagation.
use txhdl::types::{logic, Bit, Logic, I, U};

pub fn arithmetic() -> U<8> { U::<8>::new(250) + U::<8>::new(10) }
pub fn widen(x: U<8>) -> U<32> { x.resize::<32>() }
pub fn field(w: U<32>) -> U<4> { w.slice::<12, 4>() }
pub fn signed() -> I<8> { I::<8>::new(-1) + I::<8>::new(1) }

pub fn undriven_is_unknown() -> bool {
    let v = logic::Vec::<8>::default();
    let _also_a_vec: std::vec::Vec<u8> = vec![1, 2, 3];
    v.to_u().is_none() && !v.all_defined()
}

pub fn contention() -> Logic { Logic::One.resolve(Logic::Zero) }
pub fn pullup() -> Logic { Logic::Z.resolve(Logic::H) }
pub fn bit() -> Bit { !Bit::One }
