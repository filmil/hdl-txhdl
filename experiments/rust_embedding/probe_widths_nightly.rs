// SPDX-License-Identifier: Apache-2.0
// Probe 1b. The same question on nightly, with the feature enabled.
#![feature(generic_const_exprs)]
#![allow(incomplete_features)]
use txhdl::types::U;

pub fn mul<const A: usize, const B: usize>(a: U<A>, b: U<B>) -> U<{ A + B }> {
    U::new(a.raw() * b.raw())
}

pub fn call_it(a: U<32>, b: U<32>) -> U<64> { mul(a, b) }
