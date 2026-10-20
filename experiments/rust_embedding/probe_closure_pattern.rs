// SPDX-License-Identifier: Apache-2.0
// Probe 1838b. A closure in a lowered body whose parameter is a
// pattern. Expected to fail with the macro's own message, which names
// the closure and the pattern and points at a function under
// `#[lower]` (issue 1838).
use txhdl::comp::{Clock, DefaultClock, In, Out, Unit};
use txhdl::lower;
use txhdl::types::{Bit, U};

#[derive(Default)]
pub struct Check;

#[lower]
impl Unit for Check {
    async fn run(&mut self, (a, b): (In<U<8>>, In<U<8>>), ne: Out<Bit>) {
        loop {
            DefaultClock::rising().await;
            let differ = |(x, y): (U<8>, U<8>)| Bit::from(x != y);
            ne.set(differ((a.get(), b.get())));
        }
    }
}
