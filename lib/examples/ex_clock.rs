// SPDX-License-Identifier: Apache-2.0
//! The clock is in the type. A single-clock design never names one; a
//! two-clock design names exactly the second. A value goes from one to
//! the other on a channel, through `ChanCdc`, which takes the channel
//! in on one clock and gives it out on the other (issue 1017); a unit
//! that reads a port of the other clock does not compile, so the
//! crossing cannot be forgotten.
use txhdl::comp::{chan, Clock, DefaultClock, Reg, Rx, Tx};
use txhdl::types::U;
use txhdl_parts::cdc::ChanCdc;

pub struct Clk400;
impl Clock for Clk400 {
    const NAME: &'static str = "clk400";
}

/// In the default clock, and it never says so.
pub struct Counter {
    pub out: Tx<U<32>>,
    pub n: Reg<U<32>>,
}

/// In the other clock, and it says so.
pub struct Dsp {
    pub inp: Rx<U<32>, Clk400>,
}

/// Does not care which clock, and says that instead.
pub struct Probe<C: Clock> {
    pub inp: Rx<U<32>, C>,
}

impl<C: Clock> Probe<C> {
    pub fn which(&self) -> &'static str {
        C::NAME
    }
}

/// The crossing: four words of `U<32>`, two address bits and three
/// pointer bits, from the default clock to `Clk400`.
pub type Cross = ChanCdc<U<32>, 2, 4, 3, DefaultClock, Clk400>;

/// The three, and the two channel ends the crossing's `run` takes: the
/// counter's channel in, on the default clock, and the DSP's out, on
/// `Clk400`.
pub fn build() -> (Counter, Cross, (Rx<U<32>>, Tx<U<32>, Clk400>), Dsp) {
    let (tx, rx) = chan::<U<32>, DefaultClock>();
    let (tx400, rx400) = chan::<U<32>, Clk400>();
    (
        Counter {
            out: tx,
            n: Reg::new(0),
        },
        Cross::default(),
        (rx, tx400),
        Dsp { inp: rx400 },
    )
}
