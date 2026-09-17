// SPDX-License-Identifier: Apache-2.0
//! Probe 25a. Expected to fail: a field named for a reserved word. A
//! register called `next` is a VHDL signal called `next`, which VHDL
//! reserves, so the derive refuses the name at the field rather than
//! leaving the netlist not to analyse (issue 77).
use txhdl::comp::{Clock, DefaultClock, In, Out, Reg, Unit};
use txhdl::types::U;
use txhdl::{lower, Trace};

#[derive(Trace, Default)]
pub struct Counter {
    pub next: Reg<U<8>>,
}

#[lower]
impl Unit for Counter {
    async fn run(&mut self, (en,): (In<U<1>>,), (q,): (Out<U<8>>,)) {
        loop {
            DefaultClock::rising().await;
            self.next.set(self.next + en.get().zext::<8>());
            q.set(self.next);
        }
    }
}
