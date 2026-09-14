// SPDX-License-Identifier: Apache-2.0
//! Probe 15b. Expected to fail: a width nothing fixes. A slice that is
//! only compared with a number has no width for inference to find, so
//! `_` is an error, which is the reason the lowering, which sees no
//! types, asks for the width of a slice always.
use txhdl::types::U;

pub fn compared(x: U<32>) -> bool {
    x.slice::<4, _>() == 2
}
