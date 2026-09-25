// SPDX-License-Identifier: Apache-2.0
//! Helpers of `ex_helpers`, in a file of their own. `#[lower]` reads
//! the unit's file to inline a helper, and it cannot see this one, so
//! each helper here carries its own lowering, which the unit reaches by
//! the path it calls the function by (issue 504).
use txhdl::types::U;
use txhdl::{lower, Value};

/// Two bytes, the high one first.
#[derive(Value, Copy, Clone, Default, PartialEq, Debug)]
pub struct Pair {
    pub hi: U<8>,
    pub lo: U<8>,
}

/// The Gray code of a byte.
#[lower]
pub fn gray(n: U<8>) -> U<8> {
    n ^ (n >> 1u32)
}

/// A pair with its bytes swapped: a struct taken and a struct given.
#[lower]
pub fn swap(p: Pair) -> Pair {
    Pair { hi: p.lo, lo: p.hi }
}

/// The Gray codes of both bytes, mixed: a helper that calls another,
/// and reads its parameter twice, so the parameter is a wire.
#[lower]
pub fn mix(p: Pair) -> U<8> {
    let g = gray(p.hi);
    g ^ gray(p.lo)
}
