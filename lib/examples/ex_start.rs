// SPDX-License-Identifier: Apache-2.0
//! A register starts where the unit's `Default` puts it, at every
//! depth (issue 890).
//!
//! The ticker below counts from five: `Default` builds its register
//! with `Reg::new(5)`, and a reset puts it back there. Here it is not
//! the top. `Shell` holds it as a child, and the netlist is `Shell`'s,
//! with the ticker a module inside it. Nobody calls `init_reg`, which
//! names only the top's own registers and so could not reach the
//! ticker's: the generated `lowered` reads each register's start from
//! the unit's `Default`, as the run does, and the child module declares
//! its count at five and resets it there.
//!
//! The build simulates the netlist against this run under nvc and
//! Verilator, the reset included, so a count that started at zero in
//! the netlist would fail on the first edge.
use txhdl::comp::trace::{stop, Wave};
use txhdl::comp::{
    now, set_reset, signal, Clock, DefaultClock, In, Out, Reg, Running, Unit,
};
use txhdl::types::{Bit, U};
use txhdl::{lower, Trace};

// begin{unit}
/// A count of ticks, from five.
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

/// The ticker, one level down.
#[derive(Trace, Default)]
pub struct Shell {
    pub ticker: Ticker,
}

#[lower]
impl Unit<In<Bit>, Out<U<8>>> for Shell {
    async fn run(&mut self, tick: In<Bit>, total: Out<U<8>>) {
        self.ticker.run(tick, total).await;
    }
}
// end{unit}

fn main() {
    let (tick_out, tick) = signal::<Bit, DefaultClock>();
    let (total_out, total) = signal::<U<8>, DefaultClock>();
    let mut shell = Shell::default();
    if let Some(mut wave) = Wave::from_env() {
        wave.clock::<DefaultClock>();
        wave.add("tick", &tick);
        wave.add("total", &total);
        wave.add("shell", &shell);
        wave.start();
    }
    let mut sim = Running::new(shell.run(tick, total_out));

    // A cycle with no tick: the count is the register's start.
    tick_out.set(Bit::Zero);
    sim.cycle();
    println!("t={:>2} count {}", now(), total.get().raw());

    // Three ticks counted.
    tick_out.set(Bit::One);
    for _ in 0..3 {
        sim.cycle();
    }
    println!("t={:>2} count {}", now(), total.get().raw());

    // A reset puts it back to its start.
    set_reset(true);
    sim.cycle();
    println!("t={:>2} count {} (in reset)", now(), total.get().raw());
    set_reset(false);
    for _ in 0..2 {
        sim.cycle();
    }
    println!("t={:>2} count {}", now(), total.get().raw());

    // No `init_reg`: the child module's start comes from `Default`.
    let net = Shell::lowered("shell");
    print!("\n{}", net.verilog());
    stop();
    txhdl::netlist::write_vhdl_from_env(&net);
}
