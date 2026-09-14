// SPDX-License-Identifier: Apache-2.0
// Probe 20b. A crossing without a `Crossing`. Expected to fail twice: a
// named clock into another, and the default clock into a named one.
use txhdl::comp::{signal, Clock, DefaultClock, In};
use txhdl::types::U;

pub struct Clk100;
impl Clock for Clk100 {
    const NAME: &'static str = "clk100";
}
pub struct Clk400;
impl Clock for Clk400 {
    const NAME: &'static str = "clk400";
}

pub struct Dsp {
    pub inp: In<U<32>, Clk400>,
}

pub fn wrong() -> Dsp {
    let (_tx, rx) = signal::<U<32>, Clk100>();
    Dsp { inp: rx }
}

pub fn also_wrong() -> Dsp {
    let (_tx, rx) = signal::<U<32>, DefaultClock>();
    Dsp { inp: rx }
}
