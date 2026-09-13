// SPDX-License-Identifier: Apache-2.0
// Probe 9. Operator latency instead of a hand-placed stage boundary.
// Against the library's `pipeline` (async) and `funcs` (plain).
use txhdl::comp::mux;
use txhdl::funcs::{high_half, low_half};
use txhdl::pipeline::{add, mul};
use txhdl::types::{Bit, U};

pub async fn mac(a: U<32>, b: U<32>, prev: U<64>) -> U<64> {
    let p = mul(a, b).await;
    add(prev, p).await
}

pub async fn weighted(a: U<32>, b: U<32>, c: U<32>, d: U<32>) -> U<64> {
    let x = mul(a, b).await;
    let y = mul(c, d).await;
    add(x, y).await
}

/// Timeless: nothing here can span a cycle.
pub fn narrow(v: U<64>, take_low: Bit) -> U<32> {
    mux(take_low, low_half(v), high_half(v))
}
