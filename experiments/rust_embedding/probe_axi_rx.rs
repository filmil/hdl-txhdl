// SPDX-License-Identifier: Apache-2.0
//! Probe: `#[axi]` on a receiving port (#1871). Expected to fail. A
//! receiver's `ready` may wait on its `valid` under AXI, so there is
//! nothing to mark:
//!
//! ```text
//! error: `#[axi]` marks a sending port, a `Tx`, and `inp` is `Rx`: a
//!        receiver's `ready` may wait on its `valid` under AXI, so
//!        there is nothing to mark
//! ```
use txhdl::comp::{Clock, DefaultClock, Rx, Unit};
use txhdl::types::{Bit, U};
use txhdl::{lower, Trace};

#[derive(Trace, Default)]
pub struct Sink {}

#[lower]
impl Unit<Rx<U<8>>, ()> for Sink {
    #[axi(inp)]
    async fn run(&mut self, inp: Rx<U<8>>, _o: ()) {
        loop {
            DefaultClock::rising().await;
            let _ = inp.recv_if(Bit::One);
        }
    }
}
