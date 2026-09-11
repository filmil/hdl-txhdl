// SPDX-License-Identifier: Apache-2.0
//! Blinky. One register, one wire, one `when!`, and a build that says
//! how fast the clock is and how fast to blink. The LED is a square
//! wave: on for the first half of the period, off for the second.
use std::marker::PhantomData;
use txhdl::comp::{signal, Clock, DefaultClock, Module, Out, Reg, Running};
use txhdl::funcs::{eq, lt};
use txhdl::types::{Bit, U};
use txhdl::{lower, when, Trace};

/// What a blinky needs from its build. The period follows from the two
/// rates, at compile time.
pub trait BlinkyConfig: Default {
    const NAME: &'static str;
    const CLK_HZ: u64;
    const BLINK_HZ: u64;
    const PERIOD: u64 = Self::CLK_HZ / Self::BLINK_HZ;
    const HALF: u64 = Self::PERIOD / 2;
}

#[derive(Default, Trace)]
pub struct Blinky<C: BlinkyConfig> {
    pub count: Reg<U<32>>,
    _c: PhantomData<C>,
}

/// One register read per iteration, so one iteration is one cycle. The
/// LED is derived from the count rather than held in a second register,
/// which is also how a blinky is built.
#[lower]
impl<C: BlinkyConfig> Module<(), Out<Bit>> for Blinky<C> {
    async fn run(&mut self, _i: (), led: Out<Bit>) {
        loop {
            DefaultClock::rising().await;
            let n = self.count.get();
            let wrap = eq(n, U::from(C::PERIOD - 1));
            when!(wrap => {
                self.count <= 0
            } else {
                self.count <= n.wrapping_add(1)
            });
            led.set(lt(n, U::from(C::HALF)));
        }
    }
}

// A real board, and a build small enough to watch.
#[derive(Default)]
pub struct Board;
impl BlinkyConfig for Board {
    const NAME: &'static str = "board";
    const CLK_HZ: u64 = 100_000_000;
    const BLINK_HZ: u64 = 1;
}

#[derive(Default)]
pub struct Sim;
impl BlinkyConfig for Sim {
    const NAME: &'static str = "sim";
    const CLK_HZ: u64 = 16;
    const BLINK_HZ: u64 = 1;
}

fn main() {
    // The wire lives here. The blinky gets the driving end, the
    // testbench keeps the reading end, and no unit has to hold either.
    let (drive, led) = signal::<Bit, DefaultClock>();
    let mut blinky = Blinky::<Sim>::default();
    let mut sim = Running::new(blinky.run((), drive));

    // Watch the LED for two periods. The output, one character per
    // cycle:
    //
    //   sim: period 16 cycles
    //   led: ########________########________
    let mut wave = String::new();
    for _ in 0..(2 * Sim::PERIOD) {
        sim.cycle();
        wave.push(if led.get().to_bool() { '#' } else { '_' });
    }
    println!("{}: period {} cycles", Sim::NAME, Sim::PERIOD);
    println!("led: {wave}");
    // The board build is not run: a period of a hundred million cycles
    // has nothing to show in a line of text. Its number is reported.
    println!(
        "{}: period {} cycles, too long to print; {} is the build run here",
        Board::NAME,
        Board::PERIOD,
        Sim::NAME
    );

    // The same unit, lowered for each build: the period is a constant
    // of the config, and each lowering has its own.
    print!("\n{}", Blinky::<Sim>::verilog("blinky_sim"));
    print!("\n{}", Blinky::<Board>::verilog("blinky_board"));
}
