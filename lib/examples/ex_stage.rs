// SPDX-License-Identifier: Apache-2.0
//! A unit between two channels, lowered. The stage waits until a word
//! is offered and the consumer can take one, then passes the word on
//! incremented and counts it. In the lowering a channel is a
//! valid/ready handshake around its data: the wait is the guard on
//! every register drive, a receive asserts ready under it, a send
//! drives data and asserts valid under it. What comes out is a
//! combinational stage with a handshake, which is what the source says.
use txhdl::comp::trace::{stop, Wave};
use txhdl::comp::{
    chan, join2, now, until, Clock, DefaultClock, Reg, Rx, Tx, Unit,
};
use txhdl::types::U;
use txhdl::{lower, Trace};

// begin{unit}
#[derive(Trace, Default)]
pub struct Stage {
    pub count: Reg<U<8>>,
}

#[lower]
impl Unit for Stage {
    async fn run(&mut self, inp: Rx<U<8>>, out: Tx<U<8>>) {
        loop {
            until(DefaultClock::rising, || {
                inp.peek().is_some() && out.ready().to_bool()
            })
            .await;
            let v = inp.recv().unwrap_or_default();
            out.send(v + 1);
            self.count.set(self.count + 1);
        }
    }
}
// end{unit}

/// Offers a counting sequence every other cycle.
#[derive(Default)]
pub struct Source {
    pub n: Reg<U<8>>,
    pub gap: Reg<U<8>>,
}

impl Unit<(), Tx<U<8>>> for Source {
    async fn run(&mut self, _i: (), out: Tx<U<8>>) {
        loop {
            DefaultClock::rising().await;
            let gap = self.gap.get();
            self.gap.set((gap.raw() as u8 + 1) % 2);
            if gap.raw() == 0 && out.ready().to_bool() {
                out.send(self.n);
                self.n.set(self.n + 1);
            }
        }
    }
}

#[derive(Default)]
pub struct Sink;

impl Unit<Rx<U<8>>, ()> for Sink {
    async fn run(&mut self, inp: Rx<U<8>>, _o: ()) {
        loop {
            let v = inp.wait().await;
            println!("t={:>2} took {}", now(), v.raw());
        }
    }
}

fn main() {
    let (a_tx, a_rx) = chan::<U<8>, _>();
    let (b_tx, b_rx) = chan::<U<8>, _>();
    let mut source = Source::default();
    let mut stage = Stage::default();
    let mut sink = Sink;
    if let Some(mut vcd) = Wave::from_env() {
        vcd.clock::<DefaultClock>();
        vcd.add("inp", &a_rx);
        vcd.add("out", &b_rx);
        vcd.add("stage", &stage);
        vcd.start();
    }
    let mut sim = txhdl::comp::Running::new(join2(
        join2(source.run((), a_tx), stage.run(a_rx, b_tx)),
        sink.run(b_rx, ()),
    ));
    for _ in 0..6 {
        sim.cycle();
    }
    print!("\n{}", Stage::verilog("stage"));
    stop();
    txhdl::netlist::write_vhdl_from_env(&Stage::lowered("stage"));
}
