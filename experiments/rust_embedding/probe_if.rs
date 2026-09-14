// SPDX-License-Identifier: Apache-2.0
// Probe 10. `when!`, `with!` and `mux` from the library. The statement
// forms predicate the drives of one struct, its name written once,
// the condition first or the struct first; the expression form is a
// function, because there is nothing to predicate.
use txhdl::comp::{mux, rising, DefaultClock, Reg};
use txhdl::types::{Bit, U};
use txhdl::{when, with};

pub struct Unit {
    pub count: Reg<U<32>>,
    pub flag: Reg<Bit>,
}

impl Unit {
    pub async fn step(&self, enable: Bit, reset: Bit) {
        rising::<DefaultClock>().await;
        when!(enable => self {
            count: self.count + 1,
            flag: Bit::One,
        } else {
            count: U::new(0),
            flag: Bit::Zero,
        });
        with!(self <= { count: mux(reset, U::new(0), self.count.get()) });
    }
}

pub fn pick3(c1: Bit, c2: Bit, a: U<8>, b: U<8>, d: U<8>) -> U<8> {
    mux(c1, mux(c2, a, b), d)
}
