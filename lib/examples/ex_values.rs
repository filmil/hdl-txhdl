// SPDX-License-Identifier: Apache-2.0
//! Values: two-valued Bit, nine-valued Logic, and the numeric vectors.
use txhdl::types::{logic, Bit, Logic, I, U};

pub fn arithmetic() -> U<8> {
    U::<8>::new(250) + U::<8>::new(10) // wraps to 4
}

pub fn widen_and_slice(x: U<8>, w: U<32>) -> (U<32>, U<4>) {
    (x.resize::<32>(), w.slice::<12, 4>())
}

pub fn signed() -> I<8> {
    I::<8>::new(-1) + I::<8>::new(1)
}

/// An undriven vector is U, and it stays unknown through a conversion
/// rather than quietly becoming a number.
pub fn undriven_is_unknown() -> bool {
    let v = logic::Vec::<8>::default();
    v.to_u().is_none()
}

/// Two drivers on one wire, resolved by the IEEE 1164 table.
pub fn resolution() -> (Logic, Logic) {
    (Logic::One.resolve(Logic::Zero), Logic::Z.resolve(Logic::H)) // (X, H)
}

pub fn round_trip(x: U<8>) -> Option<U<8>> {
    logic::Vec::from_u(x).to_u()
}

pub fn bit() -> Bit {
    !Bit::from(true) // Zero
}
