// SPDX-License-Identifier: Apache-2.0
//! Probe: a value type with a second const parameter for its storage,
//! defaulted, `U<const N: usize, const L: usize = 1>`, so that `U<N>`
//! means what it means today and `U<256, 2>` is a value of two `u128`
//! limbs (issue 503). The question is whether the default reaches every
//! place the tree writes `U<N>`, in particular an expression path such
//! as `U::<8>::from(3u8)`, where Rust does not use a default for
//! inference.
#![allow(dead_code, clippy::needless_range_loop)]

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct U<const N: usize, const L: usize = 1>([u128; L]);

// Written out: `[u128; L]` has no `Default` for a generic `L`.
impl<const N: usize, const L: usize> Default for U<N, L> {
    fn default() -> Self {
        U([0; L])
    }
}

impl<const N: usize, const L: usize> U<N, L> {
    pub fn new(v: u128) -> Self {
        let mut l = [0u128; L];
        l[0] = v;
        U(l)
    }
}

impl<const N: usize, const L: usize> From<u8> for U<N, L> {
    fn from(v: u8) -> Self {
        Self::new(v as u128)
    }
}

impl<const N: usize, const L: usize> std::ops::Add for U<N, L> {
    type Output = Self;
    fn add(self, o: Self) -> Self {
        let mut r = [0u128; L];
        let mut carry = 0u128;
        for i in 0..L {
            let (s, c1) = self.0[i].overflowing_add(o.0[i]);
            let (s, c2) = s.overflowing_add(carry);
            r[i] = s;
            carry = (c1 || c2) as u128;
        }
        U(r)
    }
}

/// A type position: the default applies.
pub fn typed(x: U<8>) -> U<8> {
    x
}

/// A generic over the width only, as most of the tree is.
pub fn generic<const N: usize>(x: U<N>) -> U<N> {
    x + x
}

/// The forms the tree writes in expressions.
pub fn exprs() -> U<8> {
    let a = U::<8>::from(3u8);
    let b: U<8> = U::new(4);
    let c = U::<8>::new(5);
    typed(a) + b + c
}

/// A wide value of two limbs.
pub fn wide() -> U<256, 2> {
    U::<256, 2>::new(1) + U::<256, 2>::from(2u8)
}

/// A value used with nothing else to fix `L`: the case that decides
/// whether a default is taken in an expression.
pub fn unconstrained() -> u128 {
    U::<8>::from(3u8).0[0]
}
