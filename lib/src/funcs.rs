// SPDX-License-Identifier: Apache-2.0
//! Operators that cannot take a cycle. A plain `fn` has no `.await` to
//! write, so nothing here can span one, and the import says so at the
//! call site.

use crate::types::{Bit, U};

pub fn low_half(x: U<64>) -> U<32> {
    x.slice::<0, 32>()
}
pub fn high_half(x: U<64>) -> U<32> {
    x.slice::<32, 32>()
}

/// An arithmetic shift right; `>>` is the logical one.
pub fn sra<const N: usize>(a: U<N>, k: usize) -> U<N> {
    a.sra(k)
}
/// A signed compare; `<` is the unsigned one.
pub fn lt_signed<const N: usize>(a: U<N>, b: U<N>) -> Bit {
    a.lt_signed(b)
}
