// SPDX-License-Identifier: Apache-2.0
//! A unit with two processes. They take `&self`, share state through
//! registers, and `run` joins them, which is what states that they run
//! in parallel. `when!` predicates writes; it does not branch.
use txhdl::comp::{join2, Clock, DefaultClock, Module, Reg};
use txhdl::types::{Bit, U};
use txhdl::when;

pub struct DualPort {
    pub cells: Reg<U<32>>,
    pub hits_a: Reg<U<32>>,
    pub hits_b: Reg<U<32>>,
    pub enable: Reg<Bit>,
}

impl DualPort {
    /// A process loops, because a unit runs for as long as the clock
    /// does. Each iteration waits for the edge once and then reads what
    /// it needs at that edge, plainly, so one iteration is one cycle.
    async fn port_a(&self) {
        loop {
            DefaultClock::rising().await;
            let (enable, hits, cells) =
                (self.enable.get(), self.hits_a.get(), self.cells.get());
            when!(enable => {
                self.hits_a <= hits.wrapping_add(1);
                self.cells <= cells.wrapping_add(1)
            } else {
                self.hits_a <= 0
            });
        }
    }

    async fn port_b(&self) {
        loop {
            DefaultClock::rising().await;
            let hits = self.hits_b.get();
            self.hits_b.set(hits.wrapping_add(1));
        }
    }
}

impl Module<(), ()> for DualPort {
    async fn run(&mut self, _i: (), _o: ()) {
        join2(self.port_a(), self.port_b()).await;
    }
}

pub fn build() -> DualPort {
    DualPort {
        cells: Reg::new(0),
        hits_a: Reg::new(0),
        hits_b: Reg::new(0),
        enable: Reg::new(Bit::One),
    }
}
