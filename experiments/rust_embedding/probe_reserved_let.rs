// SPDX-License-Identifier: Apache-2.0
//! Probe 25b. Expected to fail: a `let` and a port named for reserved
//! words. A computed `let` is a wire of the netlist named as the `let`
//! is, and `inside` is a keyword of SystemVerilog, which Verilator
//! reads; a port called `out` is a VHDL port called `out`. Both are
//! refused where they are named (issue 77).
use txhdl::comp::{Clock, DefaultClock, In, Out, Reg, Unit};
use txhdl::types::U;
use txhdl::{lower, Trace};

#[derive(Trace, Default)]
pub struct Window {
    pub hits: Reg<U<8>>,
}

#[lower]
impl Unit for Window {
    async fn run(&mut self, (x,): (In<U<8>>,), (out,): (Out<U<8>>,)) {
        loop {
            DefaultClock::rising().await;
            let inside = (x.get() > 4) & (x.get() < 8);
            if inside {
                self.hits.set(self.hits + 1);
            }
            out.set(self.hits);
        }
    }
}
