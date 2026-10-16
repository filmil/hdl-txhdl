// SPDX-License-Identifier: Apache-2.0
//! The other side of a bundle, derived (issue 1695).
//!
//! `link::<B>()` makes both sides of a bundle in one line of a unit of
//! units, for a `B` that implements `Link`, and `Link` was written by
//! hand, with the struct the other side holds written beside it.
//! `#[derive(Link)]` writes both: `LaneHost`, each field the end opposite
//! `Lane`'s, and the `Link` between them. An array of lanes turns round
//! too, so `Lanes` holding `[Lane; 2]` gets `LanesHost` holding
//! `[LaneHost; 2]`.
//!
//! `Feed` holds the host's side and sends a count on each lane, the
//! second lane a hundred above the first, and counts the first lane's
//! acknowledgements. `Take` holds the lanes' side, keeps the last beat of
//! each and acknowledges it. `Pair` joins them with one link. Both ways
//! are channels, which are registered, so the order the two run in does
//! not matter. The run checks the beats and the acknowledgements, and the
//! build checks the netlist of `Pair` against the run under nvc and
//! Verilator.
use txhdl::comp::trace::{stop, Wave};
use txhdl::comp::{
    join2, link, signal, Clock, DefaultClock, Out, Reg, Running, Rx, Tx, Unit,
};
use txhdl::types::{Bit, U};
use txhdl::{lower, with, Link, Ports, Trace};

// begin{unit}
/// One lane: a beat in, its acknowledgement out.
#[derive(Ports, Link)]
pub struct Lane {
    pub data: Rx<U<8>>,
    pub ack: Tx<Bit>,
}

/// Two lanes, as the side that takes them holds them.
#[derive(Ports, Link)]
pub struct Lanes {
    pub lanes: [Lane; 2],
}

/// The host's side: a count on both lanes, and the first lane's
/// acknowledgements counted.
#[derive(Trace, Default)]
pub struct Feed {
    pub count: Reg<U<8>>,
    pub acks: Reg<U<8>>,
}

#[lower]
impl Unit for Feed {
    async fn run(&mut self, bus: LanesHost, acked: Out<U<8>>) {
        loop {
            DefaultClock::rising().await;
            let go = bus.lanes[0].data.ready() & bus.lanes[1].data.ready();
            if go.to_bool() {
                bus.lanes[0].data.send(self.count.get());
                bus.lanes[1].data.send(self.count.get() + 100);
            }
            let a0 = bus.lanes[0].ack.peek().is_some();
            let a1 = bus.lanes[1].ack.peek().is_some();
            let _ = bus.lanes[0].ack.recv_if(a0);
            let _ = bus.lanes[1].ack.recv_if(a1);
            with!(self <= {
                go ? count: self.count.get() + 1,
                a0 ? acks: self.acks.get() + 1,
            });
            acked.set(self.acks.get());
        }
    }
}

/// The lanes' side: the last beat of each, acknowledged.
#[derive(Trace, Default)]
pub struct Take {
    pub last0: Reg<U<8>>,
    pub last1: Reg<U<8>>,
}

#[lower]
impl Unit for Take {
    async fn run(&mut self, bus: Lanes, (got0, got1): (Out<U<8>>, Out<U<8>>)) {
        loop {
            DefaultClock::rising().await;
            let t0 =
                bus.lanes[0].data.peek().is_some() & bus.lanes[0].ack.ready();
            let t1 =
                bus.lanes[1].data.peek().is_some() & bus.lanes[1].ack.ready();
            let v0 = bus.lanes[0].data.recv_if(t0).unwrap_or_default();
            let v1 = bus.lanes[1].data.recv_if(t1).unwrap_or_default();
            if t0.to_bool() {
                bus.lanes[0].ack.send(Bit::One);
            }
            if t1.to_bool() {
                bus.lanes[1].ack.send(Bit::One);
            }
            with!(self <= {
                t0 ? last0: v0,
                t1 ? last1: v1,
            });
            got0.set(self.last0.get());
            got1.set(self.last1.get());
        }
    }
}

/// The two, joined by two lanes made whole in one line.
#[derive(Trace, Default)]
pub struct Pair {
    pub feed: Feed,
    pub take: Take,
}

#[lower]
impl Unit for Pair {
    async fn run(
        &mut self,
        _i: (),
        (acked, got0, got1): (Out<U<8>>, Out<U<8>>, Out<U<8>>),
    ) {
        let (host, lanes) = link::<Lanes>();
        join2(
            self.feed.run(host, acked),
            self.take.run(lanes, (got0, got1)),
        )
        .await;
    }
}
// end{unit}

fn main() {
    let (acked_out, acked) = signal::<U<8>, DefaultClock>();
    let (got0_out, got0) = signal::<U<8>, DefaultClock>();
    let (got1_out, got1) = signal::<U<8>, DefaultClock>();
    let mut pair = Pair::default();
    if let Some(mut w) = Wave::from_env() {
        w.clock::<DefaultClock>();
        w.add("hostgen", &pair);
        w.add("acked", &acked);
        w.add("got0", &got0);
        w.add("got1", &got1);
        w.start();
    }
    let mut sim = Running::new(pair.run((), (acked_out, got0_out, got1_out)));
    let mut seen = Vec::new();
    for cycle in 1..=10 {
        sim.cycle();
        let (a, g0, g1) =
            (acked.get().raw(), got0.get().raw(), got1.get().raw());
        println!("cycle {cycle:2}: acked {a:2}, lane 0 {g0:3}, lane 1 {g1:3}");
        seen.push((a, g0, g1));
    }
    stop();
    // A count goes out at an edge, is taken at the next, and shows from
    // the register after the third: from cycle three on, a beat a cycle
    // on each lane, lane 0 the count of three cycles before, lane 1 that
    // plus a hundred, and as many acknowledgements back as lane 0 shows.
    for (k, &got) in seen.iter().enumerate().skip(2) {
        let n = k as u128 - 2;
        assert_eq!(got, (n, n, n + 100), "cycle {}", k + 1);
    }
    let net = Pair::lowered("hostgen");
    txhdl::netlist::write_vhdl_from_env(&net);
    print!("\n{}", net.verilog());
}
