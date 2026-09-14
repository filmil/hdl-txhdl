// SPDX-License-Identifier: Apache-2.0
// Probe 20. The clock domain in the signal's type, against the library's
// `Clock`, `DefaultClock` and `Crossing`. A single-clock design never
// names a domain; a two-clock design names exactly the second.
use txhdl::comp::{signal, Clock, Crossing, DefaultClock, In, Out, Reg};
use txhdl::types::U;

pub struct Clk400;
impl Clock for Clk400 {
    const NAME: &'static str = "clk400";
}

pub struct Counter {
    pub out: Out<U<32>>,
    pub n: Reg<U<32>>,
}
pub struct Dsp {
    pub inp: In<U<32>, Clk400>,
}
pub struct Probe<C: Clock> {
    pub inp: In<U<32>, C>,
}
impl<C: Clock> Probe<C> {
    pub fn which(&self) -> &'static str {
        C::NAME
    }
}

pub fn two_clocks() -> (Counter, Crossing<U<32>, DefaultClock, Clk400>, Dsp) {
    let (tx, rx) = signal::<U<32>, DefaultClock>();
    let (xing, rx400) = Crossing::<U<32>, DefaultClock, Clk400>::new(rx);
    (
        Counter {
            out: tx,
            n: Reg::new(U::new(0)),
        },
        xing,
        Dsp { inp: rx400 },
    )
}
