// Probe 1b. The same question on nightly, with the feature enabled.
#![feature(generic_const_exprs)]
#![allow(incomplete_features)]

pub struct U<const N: usize>(pub u128);

/// The form the library wants. Stable rejects this; does nightly take it?
pub fn mul<const A: usize, const B: usize>(a: U<A>, b: U<B>) -> U<{ A + B }> {
    U(a.0 * b.0)
}

/// And does it actually resolve at a call site?
pub fn call_it(a: U<32>, b: U<32>) -> U<64> {
    mul(a, b)
}
