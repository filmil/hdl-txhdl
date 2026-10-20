// SPDX-License-Identifier: Apache-2.0
// Probe 1838a. A `move` closure in a lowered body. Expected to fail
// with the macro's own message, which names the closure and points at
// a function under `#[lower]` (issue 1838).
use txhdl::comp::{Clock, DefaultClock, In, Out, Unit};
use txhdl::lower;
use txhdl::types::{Bit, U};

#[derive(Default)]
pub struct Check;

#[lower]
impl Unit for Check {
    async fn run(&mut self, pos: In<U<11>>, hit: Out<Bit>) {
        loop {
            DefaultClock::rising().await;
            let p = pos.get();
            let at = move |k: U<11>| Bit::from(p == k);
            hit.set(at(U::<11>::from(12u16)));
        }
    }
}
