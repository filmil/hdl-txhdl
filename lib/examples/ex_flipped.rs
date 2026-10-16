// SPDX-License-Identifier: Apache-2.0
//! One bundle type for both its ends (issue 1650).
//!
//! A bundle that runs both ways, a request one way and the answer the
//! other, was two structs: each field an `In` in one and an `Out` in
//! the other, or a `Tx` and an `Rx`, written twice and kept in step by
//! hand. `Dbg<S: Side>` is written once. A field `S::Drive<T>` is an
//! `Out<T>` on the `Driver`'s side and an `In<T>` on the `Receiver`'s,
//! and `S::Recv<T>` is a channel the driver receives from: an `Rx<T>`
//! on its side, a `Tx<T>` on the other. `#[derive(Ports)]` writes `Link`
//! for both sides, so `link::<Dbg<Receiver>>()` makes the two ends at
//! once, joined.
//!
//! `Dm` holds the driver's side: it raises `haltreq` every other cycle
//! and reports what comes back on `rdata`. `Hart` holds the receiver's:
//! it counts, sends the count on `rdata`, and reports the `haltreq` it
//! reads. `Pair` joins them with one link. The answer is a channel, not
//! a wire, because `Dm` runs before `Hart` in a step, and a wire is set
//! before it is read; a channel is registered, so the order does not
//! matter. The run checks each report, and the build checks the netlist
//! of `Pair` against the run under nvc and Verilator.
use txhdl::comp::trace::{stop, Wave};
use txhdl::comp::{
    join2, link, signal, Clock, DefaultClock, Driver, Out, Receiver, Reg,
    Running, Side, Unit,
};
use txhdl::netlist::Ports as _;
use txhdl::types::{Bit, U};
use txhdl::{lower, with, Ports, Trace};

// begin{unit}
/// A debug request and its answer: one type, either side of it.
#[derive(Ports)]
pub struct Dbg<S: Side> {
    pub haltreq: S::Drive<Bit>,
    pub rdata: S::Recv<U<8>>,
}

/// The driver's side: `haltreq` every other cycle, and the last answer
/// that came back.
#[derive(Trace, Default)]
pub struct Dm {
    pub halt: Reg<Bit>,
    pub last: Reg<U<8>>,
}

#[lower]
impl Unit for Dm {
    async fn run(&mut self, dbg: Dbg<Driver>, got: Out<U<8>>) {
        loop {
            DefaultClock::rising().await;
            let came = dbg.rdata.peek().is_some();
            let answer = dbg.rdata.recv_if(came).unwrap_or_default();
            with!(self <= {
                halt: !self.halt.get(),
                came ? last: answer,
            });
            dbg.haltreq.set(self.halt.get());
            got.set(self.last.get());
        }
    }
}

/// The receiver's side: a count sent on `rdata` each cycle, and the
/// request it saw.
#[derive(Trace, Default)]
pub struct Hart {
    pub count: Reg<U<8>>,
}

#[lower]
impl Unit for Hart {
    async fn run(&mut self, dbg: Dbg<Receiver>, halted: Out<Bit>) {
        loop {
            DefaultClock::rising().await;
            let sent = dbg.rdata.ready();
            with!(self <= {
                sent ? count: self.count.get() + 1,
            });
            if sent.to_bool() {
                dbg.rdata.send(self.count.get());
            }
            halted.set(dbg.haltreq.get());
        }
    }
}

/// The two, joined by one link of the one type.
#[derive(Trace, Default)]
pub struct Pair {
    pub dm: Dm,
    pub hart: Hart,
}

#[lower]
impl Unit for Pair {
    async fn run(&mut self, _i: (), (got, halted): (Out<U<8>>, Out<Bit>)) {
        let (d, r) = link::<Dbg<Receiver>>();
        join2(self.dm.run(d, got), self.hart.run(r, halted)).await;
    }
}
// end{unit}

fn main() {
    let (got_out, got) = signal::<U<8>, DefaultClock>();
    let (halted_out, halted) = signal::<Bit, DefaultClock>();
    let mut pair = Pair::default();
    if let Some(mut w) = Wave::from_env() {
        w.clock::<DefaultClock>();
        w.add("flipped", &pair);
        w.add("got", &got);
        w.add("halted", &halted);
        w.start();
    }
    let mut sim = Running::new(pair.run((), (got_out, halted_out)));
    for cycle in 1..=8 {
        sim.cycle();
        let (g, h) = (got.get().raw(), halted.get());
        println!("cycle {cycle}: got {g}, halted {h:?}");
        // A count sent in one cycle is in the channel at its end, taken
        // by `Dm` the next, and on `got` from `last` after the edge that
        // follows: three behind. `halt` turns over every edge, so
        // `halted` is high every other cycle.
        assert_eq!(g, (cycle as u128).saturating_sub(3), "got");
        assert_eq!(h == Bit::One, cycle % 2 == 0, "halted");
    }
    stop();
    // Both sides list the same ports, by the same names, each turned
    // round.
    let d: Vec<_> = <Dbg<Driver>>::ports()
        .into_iter()
        .map(|p| (p.name, p.kind))
        .collect();
    let r: Vec<_> = <Dbg<Receiver>>::ports()
        .into_iter()
        .map(|p| (p.name, p.kind))
        .collect();
    println!("Dbg<Driver>: {d:?}\nDbg<Receiver>: {r:?}");
    let net = Pair::lowered("flipped");
    txhdl::netlist::write_vhdl_from_env(&net);
    print!("\n{}", net.verilog());
}
