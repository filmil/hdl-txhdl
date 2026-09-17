// SPDX-License-Identifier: Apache-2.0
//! Probe 25c. Expected to fail: a `let` with a register's name. The
//! computed `let pend` is a wire called `pend`, and the register is a
//! signal called `pend`, so the netlist would declare one name twice.
//! `#[lower]` does not see the struct, so the check is a constant the
//! compiler evaluates when `lowered` is compiled, and its error is at
//! the `let` (issue 77). A `let` that only reads the register,
//! `let pend = self.pend.get()`, is no wire, and is not refused.
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
