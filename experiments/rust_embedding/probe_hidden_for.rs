// SPDX-License-Identifier: Apache-2.0
// Probe for #1888. A unit with a field named `for0` and a counted
// loop, whose counter the netlist also names `for0`. Expected to fail
// with the macro's own message, which now says the loop counts a `for`
// whose body waits, rather than speaking of the wait it is at.
use txhdl::comp::{Clock, DefaultClock, Reg, Rx, Tx, Unit};
use txhdl::types::{Bit, U};
use txhdl::{lower, Trace};

#[derive(Trace, Default)]
pub struct Serial {
    pub sr: Reg<U<8>>,
    pub for0: Reg<U<3>>,
}

#[lower]
impl Unit<Rx<U<8>>, Tx<Bit>> for Serial {
    async fn run(&mut self, bytes: Rx<U<8>>, bits: Tx<Bit>) {
        loop {
            let b = bytes.wait().await;
            self.sr.set(b);
            for i in 0..8 {
                DefaultClock::rising().await;
                bits.send(self.sr.get().bit(i));
            }
        }
    }
}

pub fn netlist() -> String {
    Serial::lowered("serial").verilog()
}
