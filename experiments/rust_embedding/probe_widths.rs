// SPDX-License-Identifier: Apache-2.0
// Probe 1. Does a width that depends on arithmetic over const generic
// parameters compile on stable Rust? Against the library's own `U`.
use txhdl::types::U;

/// Widths stated by hand, as the library does it. Stable.
pub fn mul_32x32(a: U<32>, b: U<32>) -> U<64> {
    U::new(a.raw() * b.raw())
}

/// The form the library wants. Stable rejects it.
pub fn mul<const A: usize, const B: usize>(a: U<A>, b: U<B>) -> U<{ A + B }> {
    U::new(a.raw() * b.raw())
}
