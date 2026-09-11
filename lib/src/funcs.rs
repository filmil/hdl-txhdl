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

pub fn select<T: Copy>(c: Bit, a: T, b: T) -> T {
    if c.to_bool() {
        a
    } else {
        b
    }
}

pub fn is_zero<const N: usize>(x: U<N>) -> Bit {
    Bit::from_bool(x.raw() == 0)
}

pub fn eq<const N: usize>(a: U<N>, b: U<N>) -> Bit {
    Bit::from_bool(a.raw() == b.raw())
}
