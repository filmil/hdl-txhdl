// SPDX-License-Identifier: Apache-2.0
//! Probe 16. Expected to fail: a value wider than 128 bits. `U<N>` keeps
//! its bits in one `u128`, so a `U<200>` would drop the top 72 of them
//! without a word; its width is checked where it is first read, and the
//! build stops there instead. A struct holding one is refused the same
//! way, when its layout adds up the widths.
use txhdl::types::{Value, U};

pub fn wide() -> U<200> {
    U::new(1)
}

#[derive(txhdl::Value, Clone, Copy, Default)]
pub struct Wide {
    pub low: U<8>,
    pub high: U<200>,
}

pub fn layout() -> usize {
    <Wide as Value>::WIDTH
}
