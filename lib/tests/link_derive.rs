// SPDX-License-Identifier: Apache-2.0
//! `#[derive(Link)]` on a struct of ports (issue 1695): the struct the
//! other side holds, every end turned round, arrays and nested bundles
//! included, and `link::<B>()` making both in one line, in a run and in
//! a unit of units.
use txhdl::comp::trace::Kind;
use txhdl::comp::{
    join2, link, settle, Clock, DefaultClock, In, Link as _, Out, Receiver, Rx,
    Side, Unit,
};
use txhdl::types::{Bit, U};
use txhdl::{lower, Link, Ports, Trace};

/// A request and its answer, generic over its side (issue 1650).
#[derive(Ports)]
pub struct Dbg<S: Side> {
    pub halt: S::Drive<Bit>,
    pub rdata: S::Read<U<8>>,
}

/// One lane: a channel in, a wire out.
#[derive(Ports, Link)]
pub struct Lane {
    pub data: Rx<U<8>>,
    pub stall: Out<Bit>,
}

/// A bus of two lanes, a nested bundle and a wire.
#[derive(Ports, Link)]
pub struct Bus {
    pub go: In<Bit>,
    pub lanes: [Lane; 2],
    pub dbg: Dbg<Receiver>,
}

fn kinds<P: txhdl::netlist::Ports>() -> Vec<(String, Kind)> {
    P::ports().into_iter().map(|p| (p.name, p.kind)).collect()
}

fn turned(k: Kind) -> Kind {
    match k {
        Kind::In => Kind::Out,
        Kind::Out => Kind::In,
        Kind::Tx => Kind::Rx,
        Kind::Rx => Kind::Tx,
        k => k,
    }
}

/// The host's ports are the bus's, by the same names, each turned
/// round: through the array of lanes and into the nested bundle.
#[test]
fn the_host_is_the_bus_turned_round() {
    let bus = kinds::<Bus>();
    let host = kinds::<BusHost>();
    let names: Vec<_> = bus.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(
        names,
        [
            "go",
            "lanes_0_data",
            "lanes_0_stall",
            "lanes_1_data",
            "lanes_1_stall",
            "dbg_halt",
            "dbg_rdata",
        ]
    );
    let flipped: Vec<_> =
        bus.iter().map(|(n, k)| (n.clone(), turned(*k))).collect();
    assert_eq!(host, flipped);
}

/// `link` joins each end to its opposite: a wire driven on one side is
/// read on the other, and a beat sent on a lane's channel arrives on
/// that lane and no other.
#[test]
fn link_joins_every_end_to_its_opposite() {
    let (host, bus) = Bus::link();
    host.go.set(Bit::One);
    bus.lanes[1].stall.set(Bit::One);
    bus.dbg.rdata.set(U::from(7u8));
    host.dbg.halt.set(Bit::One);
    host.lanes[1].data.send(U::from(42u8));
    settle();
    assert_eq!(bus.go.get(), Bit::One);
    assert_eq!(host.lanes[1].stall.get(), Bit::One);
    assert_eq!(host.lanes[0].stall.get(), Bit::Zero);
    assert_eq!(host.dbg.rdata.get().raw(), 7);
    assert_eq!(bus.dbg.halt.get(), Bit::One);
    assert_eq!(bus.lanes[0].data.recv(), None);
    assert_eq!(bus.lanes[1].data.recv().map(|v| v.raw()), Some(42));
}

/// The host's side of a lane: it sends a count and reads the stall.
#[derive(Trace, Default)]
pub struct Feed {}

#[lower]
impl Unit for Feed {
    async fn run(&mut self, lane: LaneHost, seen: Out<Bit>) {
        loop {
            DefaultClock::rising().await;
            if lane.data.ready().to_bool() {
                lane.data.send(U::from(1u8));
            }
            seen.set(lane.stall.get());
        }
    }
}

/// The lane's own side: it takes what comes and raises the stall.
#[derive(Trace, Default)]
pub struct Take {}

#[lower]
impl Unit for Take {
    async fn run(&mut self, lane: Lane, _o: ()) {
        loop {
            DefaultClock::rising().await;
            let _ = lane.data.recv();
            lane.stall.set(Bit::One);
        }
    }
}

/// Both, joined by a lane made whole in one line.
#[derive(Trace, Default)]
pub struct Pair {
    pub feed: Feed,
    pub take: Take,
}

#[lower]
impl Unit for Pair {
    async fn run(&mut self, _i: (), seen: Out<Bit>) {
        let (host, lane) = link::<Lane>();
        join2(self.feed.run(host, seen), self.take.run(lane, ())).await;
    }
}

/// A unit of units lowers the link as a net per port, named for the
/// net and the field, and joins each child's side to them.
#[test]
fn a_derived_link_lowers_as_a_net_per_port() {
    let v = Pair::lowered("pair").verilog();
    for want in [
        ".lane_data_valid(",
        ".lane_data_ready(",
        ".lane_data_data(",
        ".lane_stall(",
    ] {
        assert!(v.contains(want), "no `{want}` in:\n{v}");
    }
}
