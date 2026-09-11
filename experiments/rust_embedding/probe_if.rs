// Probe 10. `when!` and `mux` from the library. The statement form
// predicates a list of register writes; the expression form is a
// function, because there is nothing to predicate.
use txhdl::comp::{mux, Reg};
use txhdl::types::{Bit, U};
use txhdl::when;

pub struct Unit { pub count: Reg<U<32>>, pub flag: Reg<Bit> }

impl Unit {
    pub fn step(&self, enable: Bit, reset: Bit) {
        when!(enable => {
            self.count => self.count.get().wrapping_add(U::new(1));
            self.flag  => Bit::One
        } else {
            self.count => U::new(0);
            self.flag  => Bit::Zero
        });
        self.count.set(mux(reset, U::new(0), self.count.get()));
    }
}

pub fn pick3(c1: Bit, c2: Bit, a: U<8>, b: U<8>, d: U<8>) -> U<8> { mux(c1, mux(c2, a, b), d) }
