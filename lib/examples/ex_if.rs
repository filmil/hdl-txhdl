// SPDX-License-Identifier: Apache-2.0
//! `if` in a lowered loop: Rust's `if`, over a `bool`, with the drives
//! under it written as `set`. The arms are a priority chain, the first
//! condition that holds wins, and an arm may hold another `if`, a
//! `let`, a `with!` or a `case!`. The runtime runs it as the Rust it
//! is; the lowering reads it as the chain `case!` makes, `if` and
//! `else if` in the clocked block. A `send` in an arm goes out with
//! the arm's condition as `valid`. An output is a wire, so it is
//! driven once, outside, with `mux`.
use txhdl::comp::trace::{stop, Wave};
use txhdl::comp::{
    chan, signal, Clock, DefaultClock, In, Out, Reg, Running, Tx, Unit,
};
use txhdl::types::{Bit, U};
use txhdl::{lower, Trace};

/// Counts down a length after `go`, says it is busy meanwhile, and
/// reports how many runs it has done when one ends; a reset first.
#[derive(Trace, Default)]
pub struct Pulser {
    pub count: Reg<U<4>>,
    pub busy: Reg<Bit>,
    pub runs: Reg<U<4>>,
}

#[lower]
impl Unit for Pulser {
    async fn run(
        &mut self,
        (rst, go, len): (In<Bit>, In<Bit>, In<U<4>>),
        (active, done): (Out<Bit>, Tx<U<4>>),
    ) {
        loop {
            DefaultClock::rising().await;
            let (rst, go, len) = (rst.get(), go.get(), len.get());
            if rst.to_bool() {
                self.count.set(0);
                self.busy.set(false);
                self.runs.set(0);
            } else if !self.busy.to_bool() {
                if go.to_bool() {
                    self.count.set(len);
                    self.busy.set(true);
                }
            } else if self.count == 1 {
                let n = self.runs + 1;
                self.busy.set(false);
                self.runs.set(n);
                done.send(n);
            } else {
                self.count.set(self.count - 1);
            }
            active.set(self.busy);
        }
    }
}

fn main() {
    let (rst_out, rst) = signal::<Bit, DefaultClock>();
    let (go_out, go) = signal::<Bit, DefaultClock>();
    let (len_out, len) = signal::<U<4>, DefaultClock>();
    let (active_out, active) = signal::<Bit, DefaultClock>();
    let (done_tx, done_rx) = chan::<U<4>, DefaultClock>();
    let mut pulser = Pulser::default();
    let count = pulser.count;
    if let Some(mut w) = Wave::from_env() {
        w.clock::<DefaultClock>();
        w.add("rst", &rst);
        w.add("go", &go);
        w.add("len", &len);
        w.add("pulser", &pulser);
        w.add("active", &active);
        w.add("done", &done_rx);
        w.start();
    }
    let mut sim =
        Running::new(pulser.run((rst, go, len), (active_out, done_tx)));
    // A reset, a run of three, a run of two while `go` stays high, and
    // a reset in the middle of a run of four.
    let script: [(bool, bool, u8); 16] = [
        (true, false, 0),
        (false, true, 3),
        (false, false, 3),
        (false, false, 3),
        (false, false, 3),
        (false, true, 2),
        (false, true, 2),
        (false, true, 2),
        (false, false, 2),
        (false, true, 4),
        (false, false, 4),
        (true, false, 4),
        (false, false, 4),
        (false, true, 1),
        (false, false, 1),
        (false, false, 1),
    ];
    for (r, g, l) in script {
        rst_out.set(r);
        go_out.set(g);
        len_out.set(U::from(l));
        sim.cycle();
        let done = done_rx.recv();
        println!(
            "rst={} go={} active={} count={} done={}",
            r as u8,
            g as u8,
            active.get().to_bool() as u8,
            count.get().raw(),
            done.map_or("-".to_string(), |d| d.raw().to_string())
        );
    }
    stop();
    txhdl::netlist::write_vhdl_from_env(&Pulser::lowered("pulser"));
    print!("\n{}", Pulser::verilog("pulser"));
}
