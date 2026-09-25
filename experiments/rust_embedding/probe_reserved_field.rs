// SPDX-License-Identifier: Apache-2.0
//! Probe 25a. A field named for a reserved word. A register called
//! `next` would be a VHDL signal called `next`, which VHDL reserves.
//! Until issue 497 the derive refused the name at the field, with its
//! own message, rather than leave the netlist not to analyse (issue
//! 77). Since 497 it compiles, and the netlist calls the register
//! `next_rw` in both targets.
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
