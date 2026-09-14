// SPDX-License-Identifier: Apache-2.0
// Probe 5b. The same processes taking `&mut self`. Expected to fail with
// E0499; the failure is why `Reg` is a cell and processes take `&self`.
use txhdl::comp::join2;

pub struct DualPort {
    pub hits_a: u32,
    pub hits_b: u32,
}

impl DualPort {
    async fn port_a(&mut self) {
        self.hits_a += 1
    }
    async fn port_b(&mut self) {
        self.hits_b += 1
    }
    pub async fn run(&mut self) {
        join2(self.port_a(), self.port_b()).await
    }
}
