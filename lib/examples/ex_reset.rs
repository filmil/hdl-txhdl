// SPDX-License-Identifier: Apache-2.0
//! The reset is not a port. It reaches every module the netlist
//! writes, as the clock does, and a unit that only holds registers
//! says nothing about it: while it is asserted, every register goes
//! back to what it held before the first edge, and the channels
//! between units empty.
//!
//! The ticker below counts the ticks it is given. Nothing in it
//! mentions a reset, and the netlist still gives it one. The run
//! asserts the reset for two cycles in the middle, and the count
//! starts again from zero; the trace records the reset, so the
//! simulations of the Verilog and the VHDL drive it at the same
//! cycles and must agree with the Rust.
//!
//! A unit that wants to do something under reset, rather than merely
//! forget, takes `rst: In<Bit>` and reads it, and the netlist joins
//! that port to the same net instead of adding a second. The core
//! does that: forgetting where it was is not enough, it has to fetch
//! from the reset vector.
use txhdl::comp::trace::{stop, Wave};
use txhdl::comp::{
    mux, now, set_reset, signal, Clock, DefaultClock, In, Out, Reg, Running,
    Unit,
};
use txhdl::types::{Bit, U};
use txhdl::{lower, Trace};

// begin{unit}
#[derive(Trace, Default)]
pub struct Ticker {
    pub count: Reg<U<8>>,
}

#[lower]
impl Unit<In<Bit>, Out<U<8>>> for Ticker {
    async fn run(&mut self, tick: In<Bit>, total: Out<U<8>>) {
        loop {
            DefaultClock::rising().await;
            self.count
                .set(mux(tick.get(), self.count + 1, self.count.get()));
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

    // One cycle with no tick, so that the count starting at zero is
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
    // is held, although the tick is still high.
    set_reset(true);
    for _ in 0..2 {
        sim.cycle();
        println!("t={:>2} count {} (in reset)", now(), total.get().raw());
    }

    // Released, and it counts again from there.
    set_reset(false);
    for _ in 0..3 {
        sim.cycle();
    }
    println!("t={:>2} count {}", now(), total.get().raw());

    print!("\n{}", Ticker::verilog("ticker"));
    stop();
    txhdl::netlist::write_vhdl_from_env(&Ticker::lowered("ticker"));
}
