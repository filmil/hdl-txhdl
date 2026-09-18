// SPDX-License-Identifier: Apache-2.0
//! Probe 27. Expected to compile: a function under `#[lower]` with
//! another attribute, and with a doc comment, between the marker and
//! the `fn`. Rust takes attributes in any order, and the lowering
//! finds its functions by a scan of the file, which once insisted on
//! the marker standing directly above the `fn`: the function was then
//! not inlined, and the call was refused as not lowered (issue 234).
use txhdl::comp::{Clock, DefaultClock, In, Out, Reg, Unit};
use txhdl::types::{Bit, U};
use txhdl::{lower, Trace};

#[lower]
#[allow(clippy::identity_op)]
fn plus_one(n: U<8>) -> U<8> {
    n + 1
}

#[lower]
/// The count's low bit, with the doc comment under the marker.
fn odd(n: U<8>) -> Bit {
    Bit::from(n.slice::<0, 1>() == 1)
}

#[derive(Trace, Default)]
pub struct Counter {
    pub count: Reg<U<8>>,
}

#[lower]
impl Unit for Counter {
    async fn run(&mut self, (en,): (In<Bit>,), (q, odd_q): (Out<U<8>>, Out<Bit>)) {
        loop {
            DefaultClock::rising().await;
            let n = self.count.get();
            self.count.set(plus_one(n));
            q.set(plus_one(n));
            odd_q.set(odd(n) & en.get());
        }
    }
}
