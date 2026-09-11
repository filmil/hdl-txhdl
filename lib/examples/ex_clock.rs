// SPDX-License-Identifier: Apache-2.0
//! The clock is in the type. A single-clock design never names one; a
//! two-clock design names exactly the second. A `Crossing` is the only
//! way between them, so it cannot be forgotten.
use txhdl::comp::{signal, Clock, Crossing, DefaultClock, In, Out, Reg};
use txhdl::types::U;

pub struct Clk400;
impl Clock for Clk400 {
    const NAME: &'static str = "clk400";
}

/// In the default clock, and it never says so.
pub struct Counter {
    pub out: Out<U<32>>,
    pub n: Reg<U<32>>,
}

/// In the other clock, and it says so.
pub struct Dsp {
    pub inp: In<U<32>, Clk400>,
}

/// Does not care which clock, and says that instead.
pub struct Probe<C: Clock> {
    pub inp: In<U<32>, C>,
}

impl<C: Clock> Probe<C> {
    pub fn which(&self) -> &'static str {
        C::NAME
    }
}

pub fn build() -> (Counter, Crossing<U<32>, DefaultClock, Clk400>, Dsp) {
    let (tx, rx) = signal::<U<32>, DefaultClock>();
    let (xing, rx400) = Crossing::<U<32>, DefaultClock, Clk400>::new(rx);
    (
        Counter {
            out: tx,
            n: Reg::new(0),
        },
        xing,
        Dsp { inp: rx400 },
    )
}
