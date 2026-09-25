// SPDX-License-Identifier: Apache-2.0
//! Probe: expected to fail, with the macro's own message. A register
//! of an array is chosen by a number or by a loop's variable, which
//! the lowering turns into a name; chosen by a signal, it would be a
//! multiplexer over the registers, and the lowering refuses it rather
//! than guess (issue 594).
use txhdl::comp::{Clock, DefaultClock, In, Out, Regs, Unit};
use txhdl::types::U;
use txhdl::{lower, Trace};

#[derive(Trace, Default)]
pub struct Pick {
    pub regs: Regs<U<8>, 4>,
}

#[lower]
impl Unit for Pick {
    async fn run(&mut self, sel: In<U<2>>, out: Out<U<8>>) {
        loop {
            DefaultClock::rising().await;
            let s = sel.get();
            out.set(self.regs[s].get());
        }
    }
}
