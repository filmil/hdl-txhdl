// SPDX-License-Identifier: Apache-2.0
//! A unit with two processes. They take `&self`, share state through
//! registers, and `run` joins them, which is what states that they run
//! in parallel. `when!` predicates writes; it does not branch.
use txhdl::comp::{join2, Module, Reg};
use txhdl::types::{Bit, U};
use txhdl::when;

pub struct DualPort {
    pub cells: Reg<U<32>>,
    pub hits_a: Reg<U<32>>,
    pub hits_b: Reg<U<32>>,
    pub enable: Reg<Bit>,
}

impl DualPort {
    async fn port_a(&self) {
        when!(self.enable.get() => {
            self.hits_a => self.hits_a.get().wrapping_add(U::new(1));
            self.cells  => self.cells.get().wrapping_add(U::new(1))
        } else {
            self.hits_a => U::new(0)
        });
    }

    async fn port_b(&self) {
        self.hits_b.set(self.hits_b.get().wrapping_add(U::new(1)));
    }
}

impl Module<(), ()> for DualPort {
    async fn run(&mut self, _i: (), _o: ()) {
        join2(self.port_a(), self.port_b()).await;
    }
}

pub fn build() -> DualPort {
    DualPort {
        cells: Reg::new(U::new(0)),
        hits_a: Reg::new(U::new(0)),
        hits_b: Reg::new(U::new(0)),
        enable: Reg::new(Bit::One),
    }
}
