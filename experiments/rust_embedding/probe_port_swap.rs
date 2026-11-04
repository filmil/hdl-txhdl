// SPDX-License-Identifier: Apache-2.0
//! Probe: the runtime's port and register types with their data and
//! clock parameters swapped (#1915). Expected to fail: `T` is bounded
//! by `Transaction` or `Copy` and `C` by `Clock`, so a swapped use is
//! already refused without marker types:
//!
//! ```text
//! error[E0277]: the trait bound `DefaultClock: Transaction` is not
//!        satisfied
//! error[E0277]: the trait bound `DefaultClock: Copy` is not satisfied
//! error[E0277]: the trait bound `U<8>: Clock` is not satisfied
//! ```
use txhdl::comp::{DefaultClock, In, Out, Pad, Reg, Rx, Tx};
use txhdl::types::U;

pub fn take(
    _tx: Tx<DefaultClock, U<8>>,
    _rx: Rx<DefaultClock, U<8>>,
    _in: In<DefaultClock, U<8>>,
    _out: Out<DefaultClock, U<8>>,
    _pad: Pad<DefaultClock, U<8>>,
    _reg: Reg<DefaultClock, U<8>>,
) {
}
