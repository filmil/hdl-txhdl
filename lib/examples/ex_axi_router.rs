// SPDX-License-Identifier: Apache-2.0
//! An AXI4 router: one host, three peripherals at three address
//! ranges, and a fourth address that is nobody's. The host client
//! issues five bursts without waiting for any of them; three go to
//! three different peripherals, which take different times over them,
//! and two go to the hole, which the router answers `DecErr` itself.
//!
//! The point of the run is what the clients do not say. Neither the
//! host client nor any peripheral client mentions the router, a range
//! or an identifier: they are the clients of the peer to peer
//! example, unchanged, and the router is put between them by wiring
//! alone. Each of the five channels of a link has two ends, so a
//! router is inserted by giving it the peripheral-facing ends of the
//! host's link and the host-facing ends of each peripheral's.
//!
//! The router is lowered here, and the build simulates its netlist
//! against this run under nvc and under Verilator.
use std::future::Future;
use txhdl::comp::trace::{stop, Wave};
use txhdl::comp::{join2, now, DefaultClock, Running, Unit};
use txhdl::map::AddrMap;
use txhdl::pipeline::cycles;
use txhdl::types::U;
use txhdl_parts::bus::axi::{
    axi, serve, AxiHost, AxiPer, Link, Per, Rd, Resp, Wr, Xact,
};
use txhdl_parts::bus::router::Router;

/// The link of this example: sixteen-bit addresses, thirty-two-bit
/// words, four lanes, three-bit identifiers, eight of them. Five
/// bursts are in flight at once here, and an identifier is held until
/// its answer is collected, so four would not be enough.
type HostUnit = AxiHost<16, 32, 4, 3, 8>;
type PerUnit = AxiPer<16, 32, 4, 3>;
type Job = Xact<16, 32, 4, 3>;
type End = Per<16, 32, 4, 3>;

/// The address map, a type the router is named with: three peripherals
/// of a nibble each, and every other address a hole.
struct ThreeMap;

impl AddrMap<3> for ThreeMap {
    const RANGES: [(usize, usize); 3] =
        [(0x1000, 0xf000), (0x2000, 0xf000), (0x3000, 0xf000)];
}

type Rtr = Router<3, ThreeMap, 16, 32, 4, 3>;

/// How long peripheral `k` takes over a transaction. The three differ,
/// which is what puts the answers out of the order they were issued.
fn work_cycles(k: usize) -> usize {
    [9usize, 1, 4][k]
}

/// A peripheral: it answers what it is asked after a wait of its own,
/// and a read with its own number in the high half of every word, so a
/// reply says which peripheral served it. Nothing here knows that a
/// router stands in front of it.
fn peripheral(k: usize, per: End) -> impl Future<Output = ()> {
    serve(per, 2, move |x: Job| async move {
        cycles(work_cycles(k)).await;
        match x {
            Xact::Read(rd) => {
                let addr = rd.addr().raw();
                let words: Vec<U<32>> = (0..rd.words())
                    .map(|i| {
                        let at = addr + i as u128 * 4;
                        U::from((((k as u128 + 1) << 16) | (at & 0xfff)) as u32)
                    })
                    .collect();
                rd.data(&words).await
            }
            Xact::Write(wr) => wr.ok().await,
        }
    })
}

fn main() {
    // The host's link: its host side carries the client and the
    // tracker, and its peripheral-facing ends go to the router.
    let Link {
        host,
        host_in,
        host_out,
        per_in: haw,
        per_out: hbr,
        ..
    } = axi::<16, 32, 4, 3, 8>();
    // A link per peripheral: each peripheral side carries a client and
    // a tracker, as in the peer to peer example, and its host-facing
    // ends go to the router.
    let Link {
        per: per0,
        per_in: pin0,
        per_out: pout0,
        host_in: hin0,
        host_out: hout0,
        ..
    } = axi::<16, 32, 4, 3, 8>();
    let Link {
        per: per1,
        per_in: pin1,
        per_out: pout1,
        host_in: hin1,
        host_out: hout1,
        ..
    } = axi::<16, 32, 4, 3, 8>();
    let Link {
        per: per2,
        per_in: pin2,
        per_out: pout2,
        host_in: hin2,
        host_out: hout2,
        ..
    } = axi::<16, 32, 4, 3, 8>();

    let mut host_unit = HostUnit::default();
    let mut per_unit0 = PerUnit::default();
    let mut per_unit1 = PerUnit::default();
    let mut per_unit2 = PerUnit::default();
    let mut router = Rtr::default();

    if let Some(mut w) = Wave::from_env() {
        w.clock::<DefaultClock>();
        // Every channel under the name of the router's port on it, so
        // the netlist is checked at its ports with no alias.
        w.add("aw", &haw.0);
        w.add("ar", &haw.1);
        w.add("w", &haw.2);
        w.add("b", &hbr.2);
        w.add("r", &hbr.3);
        w.add("aws_0", &hout0.0);
        w.add("ars_0", &hout0.1);
        w.add("ws_0", &hout0.2);
        w.add("bs_0", &hin0.2);
        w.add("rs_0", &hin0.3);
        w.add("aws_1", &hout1.0);
        w.add("ars_1", &hout1.1);
        w.add("ws_1", &hout1.2);
        w.add("bs_1", &hin1.2);
        w.add("rs_1", &hin1.3);
        w.add("aws_2", &hout2.0);
        w.add("ars_2", &hout2.1);
        w.add("ws_2", &hout2.2);
        w.add("bs_2", &hin2.2);
        w.add("rs_2", &hin2.3);
        w.add("router", &router);
        w.start();
    }

    // The host client: five bursts, none waited on before the next
    // goes out. Three reach peripherals, two reach the hole.
    let client = async move {
        let a = host.read(Rd::at(0x1000u32, 1)).await;
        let b = host.read(Rd::at(0x2000u32, 1)).await;
        let c = host.write(Wr::at(0x3000u32), &[U::from(0xa5a5u32)]).await;
        let d = host.read(Rd::at(0x8000u32, 2)).await;
        let e = host.write(Wr::at(0x9000u32), &[U::from(1u32)]).await;
        // Each answer says which peripheral served it, so the checks
        // are that the router sent every burst to the right range.
        for (what, want, p) in
            [("0x1000", 0x10000u128, a), ("0x2000", 0x20000, b)]
        {
            let r = p.done().await;
            println!(
                "t={:>3} read {what} -> {:?} {:#x}",
                now(),
                r.resp,
                r.data[0].raw()
            );
            assert_eq!(r.resp, Resp::Okay, "read {what}");
            assert_eq!(r.data.len(), 1, "read {what} beats");
            assert_eq!(r.data[0].raw(), want, "read {what} served wrongly");
        }
        let r = c.done().await;
        println!("t={:>3} write 0x3000 -> {:?}", now(), r.resp);
        assert_eq!(r.resp, Resp::Okay, "write to the third peripheral");
        // A burst to no peripheral's range is the router's own answer,
        // and a read of it still comes back in as many beats as it
        // asked for, so the client's collection ends.
        let r = d.done().await;
        println!(
            "t={:>3} read 0x8000 -> {:?}, {} beats",
            now(),
            r.resp,
            r.data.len()
        );
        assert_eq!(r.resp, Resp::DecErr, "a read of a hole");
        assert_eq!(r.data.len(), 2, "a read of a hole, in beats");
        let r = e.done().await;
        println!("t={:>3} write 0x9000 -> {:?}", now(), r.resp);
        assert_eq!(r.resp, Resp::DecErr, "a write to a hole");
        println!("five bursts, each answered by the right range");
    };

    let hardware = join2(
        join2(
            host_unit.run(host_in, host_out),
            router.run(
                (
                    haw.0,
                    haw.1,
                    haw.2,
                    [hin0.2, hin1.2, hin2.2],
                    [hin0.3, hin1.3, hin2.3],
                ),
                (
                    [hout0.0, hout1.0, hout2.0],
                    [hout0.1, hout1.1, hout2.1],
                    [hout0.2, hout1.2, hout2.2],
                    hbr.2,
                    hbr.3,
                ),
            ),
        ),
        join2(
            per_unit0.run(pin0, pout0),
            join2(per_unit1.run(pin1, pout1), per_unit2.run(pin2, pout2)),
        ),
    );
    let servers = join2(
        peripheral(0, per0),
        join2(peripheral(1, per1), peripheral(2, per2)),
    );

    let mut sim = Running::new(join2(hardware, join2(client, servers)));
    for _ in 0..90 {
        sim.cycle();
    }
    stop();
    let net = Rtr::lowered("axi_router");
    txhdl::netlist::write_netlists_from_env(&[&net]);
    print!("\n{}", net.verilog());
}
