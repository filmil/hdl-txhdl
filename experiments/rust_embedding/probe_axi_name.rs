// SPDX-License-Identifier: Apache-2.0
//! Probe: `#[axi]` naming no port of `run` (#1871). Expected to fail:
//!
//! ```text
//! error: `#[axi]` names `outp`, which is not a port of `run`
//! ```
use txhdl::comp::{Reg, Tx, Unit};
use txhdl::types::U;
use txhdl::{lower, Trace};

#[derive(Trace, Default)]
pub struct Count {
    pub n: Reg<U<8>>,
}

#[lower]
impl Unit<(), Tx<U<8>>> for Count {
    #[axi(outp)]
    async fn run(&mut self, _i: (), out: Tx<U<8>>) {
        loop {
            out.put(|| self.n.get()).await;
            self.n.set(self.n + 1);
        }
    }
}
