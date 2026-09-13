// SPDX-License-Identifier: Apache-2.0
// Probe 14b. Driving an input. Expected to fail: `In` has no `set`.
use txhdl::comp::In;
use txhdl::types::U;

pub fn illegal(adr: &In<U<32>>) {
    adr.set(U::new(1));
}
