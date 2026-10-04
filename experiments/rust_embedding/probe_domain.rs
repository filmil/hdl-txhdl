// SPDX-License-Identifier: Apache-2.0
// Probe 20. The clock domain in the signal's type, against the library's
// `Clock` and `DefaultClock`. A single-clock design never names a
// domain; a two-clock design names exactly the second. What crosses
// between them is a channel, through `ChanCdc` in the parts (issue
// 1017); probe 20b is what happens to a wire that tries.
use txhdl::comp::{signal, Clock, DefaultClock, In, Out, Reg};
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

/// The two, each on its own clock and each with a signal of its own.
pub fn two_clocks() -> (Counter, In<U<32>>, Out<U<32>, Clk400>, Dsp) {
    let (tx, rx) = signal::<U<32>, DefaultClock>();
    let (tx400, rx400) = signal::<U<32>, Clk400>();
    (
        Counter {
            out: tx,
            n: Reg::new(U::new(0)),
        },
        rx,
        tx400,
        Dsp { inp: rx400 },
    )
}
