// SPDX-License-Identifier: Apache-2.0
// Probe for #1888. A `#[pipeline]` latency that is not a whole number
// of cycles. Expected to fail with the macro's own message, "a latency
// is a whole number of cycles, not `1.5`". Before #1888 it was read as
// zero, and the multiplier became a combinational stage with no word.
use txhdl::pipeline;
use txhdl::types::U;

#[pipeline(mul = 1.5, add = 1)]
pub async fn mac(a: U<32>, b: U<32>, c: U<64>) -> U<64> {
    let p = mul(a, b).await;
    add(p, c).await
}
