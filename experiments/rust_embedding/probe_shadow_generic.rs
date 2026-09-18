// SPDX-License-Identifier: Apache-2.0
//! Probe 25d. Expected to compile: probe 25c's unit made generic. The
//! name of a wire is chosen by a constant, which the compiler
//! evaluates for every type the unit is lowered at, so the answer is
//! the same whether or not `lowered_verilog` below is ever called.
//! While the collision was refused rather than renamed, this was the
//! gap the refusal left: a generic unit nobody lowered was never
//! checked. That is issue 171.
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
