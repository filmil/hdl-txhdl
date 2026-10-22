// SPDX-License-Identifier: Apache-2.0
//! Probe: a `send` on a port marked `#[axi]` (#1871). Expected to fail.
//! A send offers only once there is room, which is the wait AXI forbids
//! a source, so a marked port sends with `put` alone:
//!
//! ```text
//! error: `out` is marked `#[axi]`, so it sends with `out.put(||
//!        v).await`, which offers without waiting for `ready`; a
//!        `send` offers only once there is room
//! ```
use txhdl::comp::{Clock, DefaultClock, Reg, Tx, Unit};
use txhdl::types::U;
use txhdl::{lower, Trace};

#[derive(Trace, Default)]
pub struct Count {
    pub n: Reg<U<8>>,
}

#[lower]
impl Unit<(), Tx<U<8>>> for Count {
    #[axi(out)]
    async fn run(&mut self, _i: (), out: Tx<U<8>>) {
        loop {
            DefaultClock::rising().await;
            if out.ready().to_bool() {
                out.send(self.n.get());
            }
            self.n.set(self.n + 1);
        }
    }
}
