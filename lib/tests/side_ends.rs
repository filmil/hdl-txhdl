// SPDX-License-Identifier: Apache-2.0
//! Each of a side's four ends, in a struct generic over its side with a
//! const parameter beside it, lowered in its own file and joined by
//! `link::<B<Driver>>()` (issue 1650).
use txhdl::comp::trace::Kind;
use txhdl::comp::{
    join2, link, settle, Clock, DefaultClock, Driver, Link as _, Out, Receiver,
    Side, Unit,
};
use txhdl::types::{Bit, U};
use txhdl::{lower, Ports, Trace};

/// A request and an answer on channels, a wire each way.
#[derive(Ports)]
pub struct Pkt<S: Side, const W: usize> {
    pub req: S::Send<U<W>>,
    pub ans: S::Recv<U<W>>,
    pub go: S::Drive<Bit>,
    pub busy: S::Read<Bit>,
}

fn kinds<P: txhdl::netlist::Ports>() -> Vec<(String, Kind, usize)> {
    P::ports()
        .into_iter()
        .map(|p| (p.name, p.kind, p.width))
        .collect()
}

/// The driver sends and drives, and receives and reads, each the other
/// way on the receiver's side, at the width the parameter gives.
#[test]
fn each_end_is_turned_round_at_its_width() {
    assert_eq!(
        kinds::<Pkt<Driver, 6>>(),
        [
            ("req".to_string(), Kind::Tx, 6),
            ("ans".to_string(), Kind::Rx, 6),
            ("go".to_string(), Kind::Out, 1),
            ("busy".to_string(), Kind::In, 1),
        ]
    );
    assert_eq!(
        kinds::<Pkt<Receiver, 6>>(),
        [
            ("req".to_string(), Kind::Rx, 6),
            ("ans".to_string(), Kind::Tx, 6),
            ("go".to_string(), Kind::In, 1),
            ("busy".to_string(), Kind::Out, 1),
        ]
    );
}

/// `link::<B<Driver>>()` gives the receiver's side first and the
/// driver's second, every end joined to its opposite.
#[test]
fn a_link_from_the_drivers_side_joins_every_end() {
    let (rx, tx) = Pkt::<Driver, 6>::link();
    tx.go.set(Bit::One);
    rx.busy.set(Bit::One);
    tx.req.send(U::from(5u8));
    rx.ans.send(U::from(9u8));
    settle();
    assert_eq!(rx.go.get(), Bit::One);
    assert_eq!(tx.busy.get(), Bit::One);
    assert_eq!(rx.req.recv().map(|v| v.raw()), Some(5));
    assert_eq!(tx.ans.recv().map(|v| v.raw()), Some(9));
}

/// The driver's side: a request and `go` out, the answer and `busy` in.
#[derive(Trace, Default)]
pub struct Host {}

#[lower]
impl Unit for Host {
    async fn run(&mut self, p: Pkt<Driver, 6>, seen: Out<Bit>) {
        loop {
            DefaultClock::rising().await;
            if p.req.ready().to_bool() {
                p.req.send(U::from(1u8));
            }
            let _ = p.ans.recv();
            p.go.set(Bit::One);
            seen.set(p.busy.get());
        }
    }
}

/// The receiver's side: the request in and its answer out, `go` read
/// and `busy` driven.
#[derive(Trace, Default)]
pub struct Dev {}

#[lower]
impl Unit for Dev {
    async fn run(&mut self, p: Pkt<Receiver, 6>, _o: ()) {
        loop {
            DefaultClock::rising().await;
            let take = p.req.peek().is_some() & p.ans.ready();
            let v = p.req.recv_if(take).unwrap_or_default();
            if take.to_bool() {
                p.ans.send(v);
            }
            p.busy.set(p.go.get());
        }
    }
}

/// Both, joined from the driver's side.
#[derive(Trace, Default)]
pub struct Both {
    pub host: Host,
    pub dev: Dev,
}

#[lower]
impl Unit for Both {
    async fn run(&mut self, _i: (), seen: Out<Bit>) {
        let (rx, tx) = link::<Pkt<Driver, 6>>();
        join2(self.host.run(tx, seen), self.dev.run(rx, ())).await;
    }
}

/// Each child's ports have the directions its side gives them, read by
/// the lowering from the struct in this file with the side and the width
/// put in.
#[test]
fn each_side_lowers_with_its_ends() {
    let host = Host::lowered("host").verilog();
    for want in [
        "output [5:0] p_req_data",
        "output p_req_valid",
        "input p_req_ready",
        "input [5:0] p_ans_data",
        "output p_go",
        "input p_busy",
    ] {
        assert!(host.contains(want), "no `{want}` in:\n{host}");
    }
    let dev = Dev::lowered("dev").verilog();
    for want in [
        "input [5:0] p_req_data",
        "output [5:0] p_ans_data",
        "input p_go",
        "output p_busy",
    ] {
        assert!(dev.contains(want), "no `{want}` in:\n{dev}");
    }
    let both = Both::lowered("both").verilog();
    assert!(both.contains("both_host host("), "{both}");
}
