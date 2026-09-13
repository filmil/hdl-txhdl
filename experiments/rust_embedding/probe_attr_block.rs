// SPDX-License-Identifier: Apache-2.0
// Probe 4. Is an attribute on a block expression stable?
// docs/rust-embedding.md claimed it is not, without checking.
pub fn f(x: u32) -> u32 {
    let y = #[allow(unused)] { x + 1 };
    y
}
