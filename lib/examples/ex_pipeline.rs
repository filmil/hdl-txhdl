// SPDX-License-Identifier: Apache-2.0
//! A pipeline is an async fn. Nobody places the stage boundary: it is
//! wherever an operator with latency is awaited. A plain fn cannot
//! count cycles, because it has no `.await` to write.
use txhdl::comp::mux;
use txhdl::funcs::{high_half, low_half};
use txhdl::pipeline::{add, mul};
use txhdl::types::{Bit, U};

/// Two stages, and neither boundary was placed by hand. `p` is live
/// across the await inside `mul`, so the async state machine holds it;
/// that is the work a `pipe` declaration used to ask for.
pub async fn mac(a: U<32>, b: U<32>, prev: U<64>) -> U<64> {
    let p = mul(a, b).await;
    add(prev, p).await
}

/// Timeless. Everything it calls is from `funcs`, so there is nothing
/// to await and no cycle count can be stated.
pub fn narrow(v: U<64>, low: Bit) -> U<32> {
    mux(low, low_half(v), high_half(v))
}
