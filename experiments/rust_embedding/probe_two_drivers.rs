// SPDX-License-Identifier: Apache-2.0
// Probe 16b. Two drivers on one wire. Expected to fail: `Out` is not
// `Clone` and `signal` returns one, so the second move has nothing left.
use txhdl::comp::{signal, DefaultClock, Out};
use txhdl::types::U;

pub struct Driver {
    pub out: Out<U<32>>,
}

pub fn two_drivers() -> (Driver, Driver) {
    let (tx, _rx) = signal::<U<32>, DefaultClock>();
    let a = Driver { out: tx };
    let b = Driver { out: tx };
    (a, b)
}
