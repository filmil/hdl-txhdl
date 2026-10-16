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

/// `link::<B<Receiver>>()` gives the driver's side first, the other
/// way round from the driver's own link, every end joined the same.
#[test]
fn a_link_from_the_receivers_side_joins_every_end() {
    let (tx, rx) = Pkt::<Receiver, 6>::link();
    tx.go.set(Bit::One);
    rx.busy.set(Bit::One);
    tx.req.send(U::from(3u8));
    rx.ans.send(U::from(4u8));
    settle();
    assert_eq!(rx.go.get(), Bit::One);
    assert_eq!(tx.busy.get(), Bit::One);
    assert_eq!(rx.req.recv().map(|v| v.raw()), Some(3));
    assert_eq!(tx.ans.recv().map(|v| v.raw()), Some(4));
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
    // Each child takes its own end of each channel the link made: the
    // host the request's sending end and the answer's receiving end, the
    // device the other two, and both the same two wires.
    let host = instance(&both, "both_host host(");
    let dev = instance(&both, "both_dev dev(");
    for want in [
        ".p_req_data(rx_req_tx_data)",
        ".p_ans_data(rx_ans_rx_data)",
        ".p_go(rx_go)",
        ".p_busy(rx_busy)",
    ] {
        assert!(host.contains(want), "no `{want}` in:\n{host}");
    }
    for want in [
        ".p_req_data(rx_req_rx_data)",
        ".p_ans_data(rx_ans_tx_data)",
        ".p_go(rx_go)",
        ".p_busy(rx_busy)",
    ] {
        assert!(dev.contains(want), "no `{want}` in:\n{dev}");
    }
}

/// The text of one child's instance in a netlist, from its head to the
/// `);` that ends it.
fn instance<'a>(netlist: &'a str, head: &str) -> &'a str {
    let at = netlist
        .find(head)
        .unwrap_or_else(|| panic!("no `{head}` in:\n{netlist}"));
    let rest = &netlist[at..];
    &rest[..rest.find(");").unwrap_or(rest.len())]
}

/// The side named by its path, `txhdl::comp::Driver`, as a unit may
/// write it: the lowering reads its ends as it reads `Driver`'s.
#[derive(Trace, Default)]
pub struct QHost {}

#[lower]
impl Unit for QHost {
    async fn run(&mut self, p: Pkt<txhdl::comp::Driver, 6>, seen: Out<Bit>) {
        loop {
            DefaultClock::rising().await;
            if p.req.ready().to_bool() {
                p.req.send(U::from(2u8));
            }
            let _ = p.ans.recv();
            p.go.set(Bit::One);
            seen.set(p.busy.get());
        }
    }
}

/// The qualified side lowers to the same ports as the plain one.
#[test]
fn a_side_by_its_path_lowers_as_by_its_name() {
    let q = QHost::lowered("host").verilog();
    let h = Host::lowered("host").verilog();
    let ports = |v: &str| -> Vec<String> {
        v.lines()
            .filter(|l| {
                l.trim_start().starts_with("input ")
                    || l.trim_start().starts_with("output ")
            })
            .map(str::to_string)
            .collect()
    };
    assert_eq!(ports(&q), ports(&h));
    assert!(q.contains("output p_go"), "{q}");
}
