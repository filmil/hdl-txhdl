// SPDX-License-Identifier: Apache-2.0
//! A state machine, which is what `case!` is for. A sequencer waits for
//! `go`, loads a count, runs it down and reports done, then waits again.
//! The arms are Rust patterns and the first that matches wins, so an
//! arm with a guard sits above the plain arm for the same state.
use txhdl::case;
use txhdl::comp::{signal, Clock, DefaultClock, In, Module, Out, Reg, Running};
use txhdl::types::{Bit, U};

#[derive(Copy, Clone, Default, PartialEq, Debug)]
pub enum State {
    #[default]
    Idle,
    Load,
    Run,
    Done,
}

#[derive(Default)]
pub struct Sequencer {
    pub state: Reg<State>,
    pub count: Reg<U<8>>,
}

impl Module<In<Bit>, Out<State>> for Sequencer {
    async fn run(&mut self, go: In<Bit>, observed: Out<State>) {
        loop {
            DefaultClock::rising().await;
            let (s, n) = (self.state.get(), self.count.get());
            let go = go.get();
            case!(s => {
                State::Idle if go.to_bool() => { self.state <= State::Load },
                State::Load => { self.count <= 3; self.state <= State::Run },
                State::Run if n == U::from(0) => { self.state <= State::Done },
                State::Run => { self.count <= n.wrapping_sub(1) },
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
    let mut sim = Running::new(seq.run(go_in, drive));
    let mut trace = Vec::new();
    for cycle in 0..10 {
        go.set(Bit::from_bool(cycle == 0));
        sim.cycle();
        trace.push(format!("{:?}", observed.get()));
    }
    // Prints, one state per cycle:
    //   Idle Load Run Run Run Run Done Idle Idle Idle
    println!("{}", trace.join(" "));
}
