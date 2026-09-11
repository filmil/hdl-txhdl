// SPDX-License-Identifier: Apache-2.0
//! Blinky. One register, one wire, one `when!`, and a build that says
//! how fast the clock is and how fast to blink. The LED is a square
//! wave: on for the first half of the period, off for the second.
use std::marker::PhantomData;
use txhdl::comp::{signal, Config, DefaultClock, In, Module, Out, Reg, Running};
use txhdl::config;
use txhdl::funcs::eq;
use txhdl::types::{Bit, U};
use txhdl::when;

/// What a blinky needs from its build. The period follows from the two
/// rates, at compile time.
pub trait BlinkyConfig: Config {
    const CLK_HZ: u64;
    const BLINK_HZ: u64;
    const PERIOD: u64 = Self::CLK_HZ / Self::BLINK_HZ;
    const HALF: u64 = Self::PERIOD / 2;
}

pub struct Blinky<C: BlinkyConfig> {
    pub count: Reg<U<32>>,
    _c: PhantomData<C>,
}

/// One register read per iteration, so one iteration is one cycle. The
/// LED is derived from the count rather than held in a second register,
/// which is also how a blinky is built.
impl<'a, C: BlinkyConfig> Module<(), &'a Out<Bit>> for Blinky<C> {
    async fn run(&mut self, _i: (), led: &'a Out<Bit>) {
        loop {
            let n = self.count.get().await;
            let wrap = eq(n, U::from(C::PERIOD - 1));
            when!(wrap => { self.count <= 0 } else { self.count <= n.wrapping_add(1) });
            led.set(Bit::from_bool(n.raw() < C::HALF as u128));
        }
    }
}

/// The top owns the wire and hands the blinky its driving end. The
/// reading end is public so a testbench can watch it.
pub struct Top<C: BlinkyConfig> {
    pub blinky: Blinky<C>,
    pub led: In<Bit>,
    led_drive: Out<Bit>,
}

impl<C: BlinkyConfig> Default for Top<C> {
    fn default() -> Self {
        let (led_drive, led) = signal::<Bit, DefaultClock>();
        let blinky = Blinky { count: Reg::default(), _c: PhantomData };
        Top { blinky, led, led_drive }
    }
}

impl<C: BlinkyConfig> Module<(), ()> for Top<C> {
    async fn run(&mut self, _i: (), _o: ()) {
        self.blinky.run((), &self.led_drive).await;
    }
}

// A real board, and a build small enough to watch.
config! { Board: BlinkyConfig for Top<Board> { const CLK_HZ: u64 = 100_000_000; const BLINK_HZ: u64 = 1; } }
config! { Sim:   BlinkyConfig for Top<Sim>   { const CLK_HZ: u64 = 16;          const BLINK_HZ: u64 = 1; } }

fn main() {
    // Watch the LED for two periods of the small build. `led` is cloned
    // before the run starts, because starting it borrows the top. The
    // first sample is low: nothing has driven the wire until the first
    // register read completes, which is what a reset cycle looks like.
    let mut top = Sim::top();
    let led = top.led.clone();
    let mut sim = Running::new(top.run((), ()));
    let mut wave = String::new();
    for _ in 0..(2 * Sim::PERIOD) {
        sim.cycle();
        wave.push(if led.get().to_bool() { '#' } else { '_' });
    }
    println!("{}: period {} cycles", Sim::NAME, Sim::PERIOD);
    println!("led: {wave}");
    println!("{}: period {} cycles, which is why it is not the one simulated", Board::NAME, Board::PERIOD);
}
