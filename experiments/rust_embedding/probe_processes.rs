// SPDX-License-Identifier: Apache-2.0
// Probe 5. A unit's `run` joins one async fn per process. The processes
// take `&self` and share state through the library's `Reg`.
use txhdl::comp::{join2, rising, DefaultClock, Reg, Unit};
use txhdl::types::U;

pub struct DualPort {
    pub cells: Reg<U<32>>,
    pub hits_a: Reg<U<32>>,
    pub hits_b: Reg<U<32>>,
}

impl DualPort {
    async fn port_a(&self) {
        loop {
            rising::<DefaultClock>().await;
            self.hits_a.set(self.hits_a + 1);
            self.cells.set(self.cells + 1);
        }
    }
    async fn port_b(&self) {
        loop {
            rising::<DefaultClock>().await;
            self.hits_b.set(self.hits_b + 1)
        }
    }
}

impl Unit<(), ()> for DualPort {
    async fn run(&mut self, _i: (), _o: ()) {
        join2(self.port_a(), self.port_b()).await
    }
}
