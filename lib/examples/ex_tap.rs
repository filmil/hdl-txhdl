// SPDX-License-Identifier: Apache-2.0
//! A tap between two channels, lowered: it takes a word whenever one
//! is offered and there is room to pass it on, a receive under a
//! condition, `recv_if`, so its ready is the condition and the offer;
//! and it sends on only the odd ones, a send under `when!`, so its
//! valid is the arm's condition. A unit that runs every cycle and
//! talks over channels by predicate rather than by waiting, which is
//! how a core drives a bus. Checked under nvc and Verilator against
//! its trace.
use txhdl::comp::trace::{stop, Wave};
use txhdl::comp::{chan, join2, now, Clock, DefaultClock, Reg, Rx, Tx, Unit};
use txhdl::types::U;
use txhdl::{lower, when, Trace};

// begin{unit}
#[derive(Trace, Default)]
pub struct Tap {
    pub seen: Reg<U<8>>,
    pub passed: Reg<U<8>>,
}

#[lower]
impl Unit<Rx<U<8>>, Tx<U<8>>> for Tap {
    async fn run(&mut self, inp: Rx<U<8>>, out: Tx<U<8>>) {
        loop {
            DefaultClock::rising().await;
            let offered = inp.peek().is_some();
            let room = out.ready();
            let v = inp.recv_if(room).unwrap_or_default();
            let taken = offered & room;
            let odd = taken & v.bit(0);
            when!(taken => { self.seen <= self.seen.get() + 1 });
            when!(odd => {
                out.send(v);
                self.passed <= self.passed.get() + 1
            });
        }
    }
}
// end{unit}

/// Offers a counting sequence, three words then a gap.
#[derive(Default)]
pub struct Source {
    pub n: Reg<U<8>>,
}

impl Unit<(), Tx<U<8>>> for Source {
    async fn run(&mut self, _i: (), out: Tx<U<8>>) {
        loop {
            DefaultClock::rising().await;
            let n = self.n.get();
            self.n.set(n + 1);
            if n.raw() % 4 != 3 && out.ready().to_bool() {
                out.send(n);
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
            println!("t={:>2} passed {}", now(), v.raw());
        }
    }
}

fn main() {
    let (a_tx, a_rx) = chan::<U<8>, _>();
    let (b_tx, b_rx) = chan::<U<8>, _>();
    let mut source = Source::default();
    let mut tap = Tap::default();
    let mut sink = Sink;
    if let Some(mut w) = Wave::from_env() {
        w.clock::<DefaultClock>();
        w.add("inp", &a_rx);
        w.add("out", &b_rx);
        w.add("tap", &tap);
        w.start();
    }
    let mut sim = txhdl::comp::Running::new(join2(
        join2(source.run((), a_tx), tap.run(a_rx, b_tx)),
        sink.run(b_rx, ()),
    ));
    for _ in 0..8 {
        sim.cycle();
    }
    stop();
    txhdl::netlist::write_vhdl_from_env(&Tap::lowered("tap"));
    print!("\n{}", Tap::verilog("tap"));
}
