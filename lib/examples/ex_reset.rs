// SPDX-License-Identifier: Apache-2.0
//! The reset is not a port. It reaches every module the netlist
//! writes, as the clock does, and a unit that only holds registers
//! says nothing about it: while it is asserted, every register goes
//! back to what it held before the first edge, and the channels
//! between units empty.
//!
//! The ticker below counts the ticks it is given, from five. Nothing
//! in it mentions a reset, and the netlist still gives it one. The
//! run asserts the reset for two cycles in the middle, and the count
//! starts again from five, where it started; the trace records the
//! reset, so the simulations of the Verilog and the VHDL drive it at
//! the same cycles and must agree with the Rust. The netlist takes the
//! five from the ticker's `Default`, as the run does (issue 890), and
//! its reset puts back that value and not zero (issue 728).
//!
//! The count is driven only on a tick, and the first cycle of the
//! reset has none: the reset reaches a register whether or not the
//! process drives it at that edge, as the netlist's does (issue 727).
//!
//! A unit that wants to do something under reset, rather than merely
//! forget, takes `rst: In<Bit>` and reads it, and the netlist joins
//! that port to the same net instead of adding a second. The core
//! does that: forgetting where it was is not enough, it has to fetch
//! from the reset vector.
use txhdl::comp::trace::{stop, Wave};
use txhdl::comp::{
    now, set_reset, signal, Clock, DefaultClock, In, Out, Reg, Running, Unit,
};
use txhdl::types::{Bit, U};
use txhdl::{lower, Trace};

// begin{unit}
#[derive(Trace)]
pub struct Ticker {
    pub count: Reg<U<8>>,
}

impl Default for Ticker {
    fn default() -> Self {
        Ticker {
            count: Reg::new(5u8),
        }
    }
}

#[lower]
impl Unit<In<Bit>, Out<U<8>>> for Ticker {
    async fn run(&mut self, tick: In<Bit>, total: Out<U<8>>) {
        loop {
            DefaultClock::rising().await;
            if tick.get().to_bool() {
                self.count.set(self.count + 1);
            }
            total.set(self.count);
        }
    }
}
// end{unit}

fn main() {
    let (tick_out, tick) = signal::<Bit, DefaultClock>();
    let (total_out, total) = signal::<U<8>, DefaultClock>();
    let mut ticker = Ticker::default();
    if let Some(mut wave) = Wave::from_env() {
        wave.clock::<DefaultClock>();
        wave.add("tick", &tick);
        wave.add("total", &total);
        wave.add("ticker", &ticker);
        wave.start();
    }
    let mut sim = Running::new(ticker.run(tick, total_out));

    // One cycle with no tick, so that the count starting at five is
    // the register's own value rather than something not yet counted,
    // and so that the waveform has an edge to draw the tick from.
    tick_out.set(Bit::Zero);
    sim.cycle();

    // Four ticks counted.
    tick_out.set(Bit::One);
    for _ in 0..4 {
        sim.cycle();
    }
    println!("t={:>2} count {}", now(), total.get().raw());

    // The reset, asserted for two cycles. The count goes back to what
    // it was before the first edge, and stays there while the reset
    // is held: at the first edge with no tick, so nothing drives it,
    // and at the second with the tick high again.
    set_reset(true);
    for high in [Bit::Zero, Bit::One] {
        tick_out.set(high);
        sim.cycle();
        println!("t={:>2} count {} (in reset)", now(), total.get().raw());
    }

    // Released, and it counts again from there.
    set_reset(false);
    for _ in 0..3 {
        sim.cycle();
    }
    println!("t={:>2} count {}", now(), total.get().raw());

    // The netlist starts the count at five, as `Default` does.
    let net = Ticker::lowered("ticker");
    print!("\n{}", net.verilog());
    stop();
    txhdl::netlist::write_vhdl_from_env(&net);
}
