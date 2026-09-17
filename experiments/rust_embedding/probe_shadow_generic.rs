// SPDX-License-Identifier: Apache-2.0
//! Probe 25d. Expected to fail: probe 25c's unit made generic. The
//! check on a `let` with a register's name is a constant evaluated when
//! `lowered` is compiled, so for a generic unit it runs only for a type
//! that is lowered: without `lowered_verilog` below this file builds.
//! That gap is issue 171.
use txhdl::comp::{Clock, DefaultClock, In, Out, Reg, Unit};
use txhdl::types::{Bit, U};
use txhdl::{lower, Trace};

#[derive(Trace, Default)]
pub struct Latch<const W: usize> {
    pub pend: Reg<Bit>,
}

#[lower]
impl<const W: usize> Unit for Latch<W> {
    async fn run(&mut self, (set,): (In<Bit>,), (q,): (Out<U<1>>,)) {
        loop {
            DefaultClock::rising().await;
            let pend = self.pend.get() | set.get();
            self.pend.set(pend);
            q.set(self.pend.get().zext::<1>());
        }
    }
}

/// The one type lowered, which is what evaluates the check.
pub fn lowered_verilog() -> String {
    Latch::<1>::verilog("latch")
}
