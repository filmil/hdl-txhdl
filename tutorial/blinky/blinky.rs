// SPDX-License-Identifier: Apache-2.0
//! A blinky. One register counts the cycles of a period; the LED is
//! high for the first half of it and low for the second. Run, it
//! prints the LED as a line of characters, one per cycle; run with
//! `--verilog`, it prints the netlist instead.
use txhdl::comp::{signal, Clock, DefaultClock, Out, Reg, Running, Unit};
use txhdl::types::{Bit, U};
use txhdl::{lower, when, Trace};

/// Cycles per blink. Sixteen, so that a run fits on a line; a board
/// at 100 MHz blinking once a second says 100_000_000 here.
const PERIOD: u32 = 16;

#[derive(Default, Trace)]
pub struct Blinky {
    pub count: Reg<U<32>>,
}

/// Every iteration waits for the clock's rising edge, so one
/// iteration is one cycle. The count wraps at the period; the LED is
/// a compare on the count, a wire and not a register.
#[lower]
impl Unit for Blinky {
    async fn run(&mut self, _inputs: (), led: Out<Bit>) {
        loop {
            DefaultClock::rising().await;
            when!(self.count == PERIOD - 1 => self { count: 0 } else {
                count: self.count + 1
            });
            led.set(self.count < PERIOD / 2);
        }
    }
}

fn main() {
    if std::env::args().any(|a| a == "--verilog") {
        print!("{}", Blinky::verilog("blinky"));
        return;
    }
    // A wire has two ends: the blinky drives one, main reads the other.
    let (drive, led) = signal::<Bit, DefaultClock>();
    let mut blinky = Blinky::default();
    let mut sim = Running::new(blinky.run((), drive));
    let mut line = String::new();
    for _ in 0..2 * PERIOD {
        sim.cycle();
        line.push(if led.get().to_bool() { '#' } else { '_' });
    }
    println!("led: {line}");
}
