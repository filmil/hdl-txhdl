// SPDX-License-Identifier: Apache-2.0
// Probe for #1888. `ex_hold`'s stepper with a field named `busy_held`,
// the register the netlist keeps the output `busy` in. Expected to
// fail with the macro's own message, which says it is what port `busy`
// was last set to, not the wait the loop is at.
use txhdl::comp::{until, Clock, DefaultClock, In, Out, Reg, Rx, Tx, Unit};
use txhdl::types::{Bit, U};
use txhdl::{lower, Trace};

#[derive(Trace, Default)]
pub struct Stepper {
    pub held: Reg<U<8>>,
    pub busy_held: Reg<Bit>,
}

#[lower]
impl Unit<(Rx<U<8>>, In<Bit>), (Tx<U<8>>, Out<Bit>)> for Stepper {
    async fn run(
        &mut self,
        (words, go): (Rx<U<8>>, In<Bit>),
        (sums, busy): (Tx<U<8>>, Out<Bit>),
    ) {
        loop {
            let w = words.wait().await;
            self.held.set(w);
            busy.set(Bit::One);
            DefaultClock::rising().await;
            self.held.set(self.held + 1);
            until(DefaultClock::rising, || go.get().to_bool()).await;
            sums.send(self.held.get());
            busy.set(Bit::Zero);
        }
    }
}

pub fn netlist() -> String {
    Stepper::lowered("stepper").verilog()
}
