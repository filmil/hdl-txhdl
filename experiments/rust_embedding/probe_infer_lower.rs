// SPDX-License-Identifier: Apache-2.0
//! Probe 15c. Expected to fail: a width the lowering needs, left to
//! inference. Rust would infer this slice's width from the drive it
//! feeds, but the lowering sees tokens, not types, and asks for it.
use txhdl::comp::{Clock, DefaultClock, In, Out, Reg, Unit};
use txhdl::types::U;
use txhdl::{lower, Trace};

#[derive(Trace, Default)]
pub struct Low {
    pub low: Reg<U<8>>,
}

#[lower]
impl Unit<In<U<32>>, Out<U<8>>> for Low {
    async fn run(&mut self, x: In<U<32>>, y: Out<U<8>>) {
        loop {
            DefaultClock::rising().await;
            self.low.set(x.get().slice::<0, _>());
            y.set(self.low);
        }
    }
}
