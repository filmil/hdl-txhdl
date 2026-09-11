// SPDX-License-Identifier: Apache-2.0
//! A structural netlist, from the same walk the waveform uses. The
//! units here hold their ports as fields, so the walk sees which end
//! of the wire each holds, and the skeleton has a port on each, a wire
//! in the top, and an instance per child. A unit that takes its ports
//! as parameters of `run` shows its registers and nothing else, which
//! is the gap the proc-macro lowering closes. The bodies are empty:
//! this is the structure, and behaviour is the next experiment. The
//! design is also run, so it has a waveform.
use txhdl::comp::trace::Vcd;
use txhdl::comp::{
    join2, signal, Clock, DefaultClock, In, Module, Out, Reg, Running,
};
use txhdl::netlist;
use txhdl::types::U;
use txhdl::Trace;

#[derive(Trace)]
pub struct Producer {
    pub n: Reg<U<8>>,
    pub out: Out<U<8>>,
}

impl Module<(), ()> for Producer {
    async fn run(&mut self, _i: (), _o: ()) {
        loop {
            DefaultClock::edge().await;
            let n = self.n.get();
            self.out.set(n);
            self.n.set(n.wrapping_add(1));
        }
    }
}

#[derive(Trace)]
pub struct Consumer {
    pub total: Reg<U<8>>,
    pub inp: In<U<8>>,
}

impl Module<(), ()> for Consumer {
    async fn run(&mut self, _i: (), _o: ()) {
        loop {
            DefaultClock::edge().await;
            let total = self.total.get();
            self.total.set(total.wrapping_add(self.inp.get()));
        }
    }
}

/// The top splits the wire and hands each child its end at
/// construction, so both ends are fields below the top.
#[derive(Trace)]
pub struct Top {
    pub producer: Producer,
    pub consumer: Consumer,
}

impl Default for Top {
    fn default() -> Self {
        let (out, inp) = signal::<U<8>, DefaultClock>();
        Top {
            producer: Producer {
                n: Reg::default(),
                out,
            },
            consumer: Consumer {
                total: Reg::default(),
                inp,
            },
        }
    }
}

impl Module<(), ()> for Top {
    async fn run(&mut self, _i: (), _o: ()) {
        join2(self.producer.run((), ()), self.consumer.run((), ())).await;
    }
}

fn main() {
    let mut top = Top::default();
    print!("{}", netlist::verilog("top", &top));
    if let Some(mut vcd) = Vcd::from_env() {
        vcd.clock::<DefaultClock>();
        vcd.add("top", &top);
        vcd.start();
    }
    let mut sim = Running::new(top.run((), ()));
    for _ in 0..6 {
        sim.step();
    }
}
