// SPDX-License-Identifier: Apache-2.0
//! A register reads in place. It is `Copy`, and it has the operators
//! and the conversions of its value, so `self.acc + x`, `self.n == 7`,
//! `!self.top` and `sum.set(self.acc)` say what `get` said, the value
//! the last edge latched, without a `let` at the top of the step. A
//! drive in the same step does not change what a read sees, as in
//! hardware. `get` is still there for a method of the value, a typed
//! argument such as a `mux` arm, and a `case!`.
use txhdl::comp::trace::{stop, Wave};
use txhdl::comp::{
    mux, signal, Clock, DefaultClock, In, Out, Reg, Running, Unit,
};
use txhdl::types::{Bit, U};
use txhdl::{lower, Trace};

/// Sums eight inputs into `acc`, then holds the sum and says so on
/// `full` until the next clear; `n` counts the inputs taken.
#[derive(Trace, Default)]
pub struct Window {
    pub acc: Reg<U<8>>,
    pub n: Reg<U<3>>,
    pub top: Reg<Bit>,
}

#[lower]
impl Unit for Window {
    async fn run(
        &mut self,
        (x, en, clear): (In<U<8>>, In<Bit>, In<Bit>),
        (sum, full): (Out<U<8>>, Out<Bit>),
    ) {
        loop {
            DefaultClock::rising().await;
            if clear.get().to_bool() {
                self.acc.set(0);
                self.n.set(0);
                self.top.set(false);
            } else if en.get().to_bool() & !self.top.to_bool() {
                self.acc.set(self.acc + x.get());
                self.n.set(self.n + 1);
                self.top.set(self.n == 7);
            }
            sum.set(mux(self.top, self.acc.get(), self.acc & 0xF0));
            full.set(self.top);
        }
    }
}

fn main() {
    let (x_out, x) = signal::<U<8>, DefaultClock>();
    let (en_out, en) = signal::<Bit, DefaultClock>();
    let (clear_out, clear) = signal::<Bit, DefaultClock>();
    let (sum_out, sum) = signal::<U<8>, DefaultClock>();
    let (full_out, full) = signal::<Bit, DefaultClock>();
    let mut window = Window::default();
    let (acc, n) = (window.acc, window.n);
    if let Some(mut w) = Wave::from_env() {
        w.clock::<DefaultClock>();
        w.add("x", &x);
        w.add("en", &en);
        w.add("clear", &clear);
        w.add("window", &window);
        w.add("sum", &sum);
        w.add("full", &full);
        w.start();
    }
    let mut sim = Running::new(window.run((x, en, clear), (sum_out, full_out)));
    // A clear, eleven inputs of which the eighth fills the window and
    // the full window ignores the rest, then a clear and one input.
    for c in 0..14u8 {
        x_out.set(U::from(c * 3 + 1));
        en_out.set(c != 0 && c != 12);
        clear_out.set(c == 0 || c == 12);
        sim.cycle();
        println!(
            "x={:3} acc={:3} n={} sum={:3} full={}",
            c * 3 + 1,
            acc.get().raw(),
            n.get().raw(),
            sum.get().raw(),
            full.get().to_bool() as u8
        );
    }
    stop();
    txhdl::netlist::write_vhdl_from_env(&Window::lowered("window"));
    print!("\n{}", Window::verilog("window"));
}
