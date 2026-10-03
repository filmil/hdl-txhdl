// SPDX-License-Identifier: Apache-2.0
//! Probe: a send after a plain `C::rising()`, under no condition
//! (issue 882). Expected to fail. A send drives `valid` with the
//! condition it is under, and a plain wait gives none, so the send
//! would offer whether or not the channel has room, which the run
//! refuses as well: `Tx::send` panics on a full channel. The message
//! says what the send needs:
//!
//! ```text
//! error: a send needs a condition for its `valid`: put it under
//!        `if out.ready().to_bool()`, wait with
//!        `until(C::rising, || out.ready().to_bool())`, or use
//!        `out.put(|| v).await`
//! ```
//!
//! It used to say "send needs a wait before it", of a send that had a
//! wait before it.
use txhdl::comp::{Clock, DefaultClock, Reg, Tx, Unit};
use txhdl::types::U;
use txhdl::{lower, Trace};

#[derive(Trace, Default)]
pub struct Count {
    pub n: Reg<U<8>>,
}

#[lower]
impl Unit<(), Tx<U<8>>> for Count {
    async fn run(&mut self, _i: (), out: Tx<U<8>>) {
        loop {
            DefaultClock::rising().await;
            out.send(self.n.get());
            self.n.set(self.n + 1);
        }
    }
}
