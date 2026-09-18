// SPDX-License-Identifier: Apache-2.0
//! Probe 25c. Expected to compile: a `let` with a register's name.
//! The computed `let pend` wants a wire called `pend`, and the
//! register is a signal called `pend`, so the netlist would declare
//! one name twice. It was refused for that reason (issue 77); since
//! issue 171 the wire takes the name `pend_w` instead, chosen by a
//! constant, since `#[lower]` does not see the struct, and the
//! netlist says which `let` became which wire. A `let` that only
//! reads the register, `let pend = self.pend.get()`, is no wire and
//! takes no name at all.
use txhdl::comp::{Clock, DefaultClock, In, Out, Reg, Unit};
use txhdl::types::{Bit, U};
use txhdl::{lower, Trace};

#[derive(Trace, Default)]
pub struct Latch {
    pub pend: Reg<Bit>,
}

#[lower]
impl Unit for Latch {
    async fn run(&mut self, (set,): (In<Bit>,), (q,): (Out<U<1>>,)) {
        loop {
            DefaultClock::rising().await;
            let pend = self.pend.get() | set.get();
            self.pend.set(pend);
            q.set(self.pend.get().zext::<1>());
        }
    }
}
