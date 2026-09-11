// SPDX-License-Identifier: Apache-2.0
//! Operators that may take a cycle. Every one is `async`, and every one
//! is a pipeline stage the moment it is awaited. They live here and not
//! beside the combinational ones so that the module a design imports
//! from says whether an operation can cost time.
//!
//! There is no `impl Add`. An operator must return a value, and an
//! operation whose latency the mapping decides cannot. `.await` is not a
//! claim that a cycle is spent; it is a refusal to decide here.
//!
//! Widths are declared per function rather than computed, because
//! `U<{A + B}>` needs nightly Rust. The prototype pays that in
//! functions; a nightly build could pay it once.

use crate::types::U;

pub async fn mul(a: U<32>, b: U<32>) -> U<64> {
    U::new(a.raw() * b.raw())
}

pub async fn add<const N: usize>(a: U<N>, b: U<N>) -> U<N> {
    a.wrapping_add(b)
}

pub async fn sub<const N: usize>(a: U<N>, b: U<N>) -> U<N> {
    a.wrapping_sub(b)
}

pub async fn div(a: U<32>, b: U<32>) -> U<32> {
    if b.raw() == 0 { U::new(0) } else { U::new(a.raw() / b.raw()) }
}

/// Wait a stated number of cycles. This is the one place a design counts
/// them, and it is honoured exactly: a baud interval comes from the wire.
pub async fn cycles(_n: usize) {}
