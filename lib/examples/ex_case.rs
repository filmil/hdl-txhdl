// SPDX-License-Identifier: Apache-2.0
//! A state machine, which is what `case!` is for. A sequencer waits for
//! `go`, loads a count, runs it down and reports done, then waits again.
//! The arms are Rust patterns and the first that matches wins, so an
//! arm with a guard sits above the plain arm for the same state.
use txhdl::comp::trace::{stop, Wave};
use txhdl::comp::{signal, Clock, DefaultClock, In, Out, Reg, Running, Unit};
use txhdl::types::{Bit, U};
use txhdl::{case, lower, Trace, Value};

#[derive(Value, Copy, Clone, Default, PartialEq, Debug)]
pub enum State {
    #[default]
    Idle,
    Load,
    Run,
    Done,
}

#[derive(Trace, Default)]
pub struct Sequencer {
    pub state: Reg<State>,
    pub count: Reg<U<8>>,
}

#[lower]
impl Unit<In<Bit>, Out<State>> for Sequencer {
    async fn run(&mut self, go: In<Bit>, observed: Out<State>) {
        loop {
            DefaultClock::rising().await;
            let (s, n) = (self.state.get(), self.count.get());
            let go = go.get();
            case!(s => {
                State::Idle if go.to_bool() => { self.state <= State::Load },
                State::Load => { self.count <= 3; self.state <= State::Run },
                State::Run if n == 0 => { self.state <= State::Done },
                State::Run => { self.count <= n - 1 },
                State::Done | State::Idle => { self.state <= State::Idle },
            });
            observed.set(s);
        }
    }
}

fn main() {
    // Raise `go` for one cycle, then watch the state.
    let (go, go_in) = signal::<Bit, DefaultClock>();
    let (drive, observed) = signal::<State, DefaultClock>();
    let mut seq = Sequencer::default();
    if let Some(mut w) = Wave::from_env() {
        w.clock::<DefaultClock>();
        w.add("go", &go_in);
        w.add("observed", &observed);
        w.add("sequencer", &seq);
        w.start();
    }
    let mut sim = Running::new(seq.run(go_in, drive));
    let mut trace = Vec::new();
    for cycle in 0..10 {
        go.set(cycle == 0);
        sim.cycle();
        trace.push(format!("{:?}", observed.get()));
    }
    // Prints, one state per cycle:
    //   Idle Load Run Run Run Run Done Idle Idle Idle
    stop();
    println!("{}", trace.join(" "));

    // The state machine, lowered: each arm a condition on the state
    // register, in order, the first that holds winning.
    print!("\n{}", Sequencer::verilog("sequencer"));
    txhdl::netlist::write_vhdl_from_env(&Sequencer::lowered("sequencer"));
}
