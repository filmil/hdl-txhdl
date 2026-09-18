// SPDX-License-Identifier: Apache-2.0
//! An AXI4 arbiter: two hosts, one memory, and the answers finding
//! their way home.
//!
//! The arbiter is the router's mirror. The router takes one host and
//! several peripherals and decides on the address; the arbiter takes
//! several hosts and one peripheral and decides whose turn it is. Put
//! the two together and a design has a bus rather than a wire.
//!
//! The hard part is not the deciding. It is that an answer has to
//! reach the host that asked, and the peripheral does not know there
//! are two. AXI4 carries an identifier from each address phase back
//! on its answer, so the arbiter writes the host's port number above
//! the host's own identifier on the way out and takes it off again on
//! the way back. That is why the two sides have different identifier
//! widths here: two bits on each host, five on the peripheral side.
//!
//! The other hard part is the write data channel, which carries no
//! identifier at all. A beat belongs to the oldest address phase that
//! has not finished, so the arbiter holds that channel for the burst
//! it granted until the last beat. Both hosts below write four beats
//! at the same time, and the memory sees two whole bursts rather than
//! a mixture.
//!
//! Neither client mentions the arbiter, a port number or an
//! identifier. They are the clients of the peer to peer example,
//! unchanged, and the arbiter is put between them by wiring alone.
//!
//! The arbiter is lowered here, and the build simulates its netlist
//! against this run under nvc and under Verilator.
use std::cell::RefCell;
use std::future::Future;
use std::rc::Rc;
use txhdl::comp::trace::{stop, Wave};
use txhdl::comp::{join2, now, DefaultClock, Running, Unit};
use txhdl::types::U;
use txhdl_parts::bus::arbiter::Arbiter2;
use txhdl_parts::bus::axi::{
    axi, serve, AxiHost, AxiPer, Link, Per, Rd, Resp, Wr, Xact,
};

/// A host's link: sixteen-bit addresses, thirty-two-bit words, four
/// lanes, two-bit identifiers, four of them.
type HostUnit = AxiHost<16, 32, 4, 2, 4>;
/// The peripheral side's, whose identifier is three bits wider: two
/// for a host's own, and three above them for the port number.
type PerUnit = AxiPer<16, 32, 4, 5>;
type Job = Xact<16, 32, 4, 5>;
type End = Per<16, 32, 4, 5>;

/// Two hosts, identifiers two bits wide on their side and five on the
/// peripheral's, round robin rather than fixed priority.
type Arb = Arbiter2<16, 32, 4, 2, 5, 0>;

/// The memory behind the arbiter. It answers a read with the address
/// it was asked for, so an answer says which burst it belongs to, and
/// it records every write burst whole, so the run can show that the
/// beats of two did not mix. Nothing here knows there are two hosts.
fn memory(
    per: End,
    seen: Rc<RefCell<Vec<(u128, Vec<u128>)>>>,
) -> impl Future<Output = ()> {
    serve(per, 4, move |x: Job| {
        let seen = seen.clone();
        async move {
            match x {
                Xact::Read(rd) => {
                    let at = rd.addr().raw();
                    let words: Vec<U<32>> = (0..rd.words())
                        .map(|i| U::from((at + i as u128 * 4) as u32))
                        .collect();
                    rd.data(&words).await
                }
                Xact::Write(wr) => {
                    let beats: Vec<u128> =
                        wr.data().iter().map(|w| w.raw()).collect();
                    seen.borrow_mut().push((wr.addr().raw(), beats));
                    wr.ok().await
                }
            }
        }
    })
}

fn main() {
    // What `arbiter!` wrote for two hosts, for the document to show
    // without a hand-typed copy.
    if std::env::args().any(|a| a == "--source") {
        print!("{}", txhdl_parts::bus::arbiter::arbiter2::SOURCE);
        return;
    }
    // A link per host: each host side carries a client and a tracker,
    // and its peripheral-facing ends go to the arbiter.
    let l0 = axi::<16, 32, 4, 2, 4>();
    let l1 = axi::<16, 32, 4, 2, 4>();
    // The memory's link, on the wider identifier.
    let lp = axi::<16, 32, 4, 5, 8>();

    let mut host0 = HostUnit::default();
    let mut host1 = HostUnit::default();
    let mut per_unit = PerUnit::default();
    let mut arb = Arb::default();

    if let Some(mut w) = Wave::from_env() {
        w.clock::<DefaultClock>();
        // Every channel under the name of the arbiter's port on it,
        // so the netlist is checked at its ports with no alias.
        w.add("aw0", &l0.per_in.0);
        w.add("ar0", &l0.per_in.1);
        w.add("w0", &l0.per_in.2);
        w.add("b0", &l0.per_out.2);
        w.add("r0", &l0.per_out.3);
        w.add("aw1", &l1.per_in.0);
        w.add("ar1", &l1.per_in.1);
        w.add("w1", &l1.per_in.2);
        w.add("b1", &l1.per_out.2);
        w.add("r1", &l1.per_out.3);
        w.add("aw", &lp.host_out.0);
        w.add("ar", &lp.host_out.1);
        w.add("w", &lp.host_out.2);
        w.add("b", &lp.host_in.2);
        w.add("r", &lp.host_in.3);
        w.add("arbiter", &arb);
        w.start();
    }

    let seen = Rc::new(RefCell::new(Vec::new()));
    // `Host` is not `Clone`, and does not need to be: each client
    // owns its own end.
    let (h0, h1) = (l0.host, l1.host);

    // The first host: two reads, then a write of four beats.
    let first = async move {
        let a = h0.read(Rd::at(0x100u32, 2)).await;
        let b = h0.read(Rd::at(0x200u32, 2)).await;
        let c = h0
            .write(
                Wr::at(0x1000u32),
                &[
                    U::from(0xa0u32),
                    U::from(0xa1u32),
                    U::from(0xa2u32),
                    U::from(0xa3u32),
                ],
            )
            .await;
        let ra = a.done().await;
        let rb = b.done().await;
        let rc = c.done().await;
        println!(
            "{:3}  host 0 read {:#06x} and {:#06x}, wrote four beats: {:?}",
            now(),
            ra.data[0].raw(),
            rb.data[0].raw(),
            rc.resp
        );
        assert_eq!(ra.data[0].raw(), 0x100, "its own first read");
        assert_eq!(rb.data[0].raw(), 0x200, "its own second read");
        assert_eq!(rc.resp, Resp::Okay);
    };

    // The second host, at the same time, asking for other addresses.
    let second = async move {
        let a = h1.read(Rd::at(0x900u32, 2)).await;
        let b = h1.read(Rd::at(0xa00u32, 2)).await;
        let c = h1
            .write(
                Wr::at(0x2000u32),
                &[
                    U::from(0xb0u32),
                    U::from(0xb1u32),
                    U::from(0xb2u32),
                    U::from(0xb3u32),
                ],
            )
            .await;
        let ra = a.done().await;
        let rb = b.done().await;
        let rc = c.done().await;
        println!(
            "{:3}  host 1 read {:#06x} and {:#06x}, wrote four beats: {:?}",
            now(),
            ra.data[0].raw(),
            rb.data[0].raw(),
            rc.resp
        );
        assert_eq!(ra.data[0].raw(), 0x900, "its own first read");
        assert_eq!(rb.data[0].raw(), 0xa00, "its own second read");
        assert_eq!(rc.resp, Resp::Okay);
    };

    let mut sim = Running::new(join2(
        join2(
            join2(
                host0.run(l0.host_in, l0.host_out),
                host1.run(l1.host_in, l1.host_out),
            ),
            join2(
                per_unit.run(lp.per_in, lp.per_out),
                arb.run(
                    (
                        l0.per_in.0,
                        l0.per_in.1,
                        l0.per_in.2,
                        l1.per_in.0,
                        l1.per_in.1,
                        l1.per_in.2,
                        lp.host_in.2,
                        lp.host_in.3,
                    ),
                    (
                        lp.host_out.0,
                        lp.host_out.1,
                        lp.host_out.2,
                        l0.per_out.2,
                        l0.per_out.3,
                        l1.per_out.2,
                        l1.per_out.3,
                    ),
                ),
            ),
        ),
        join2(memory(lp.per, seen.clone()), join2(first, second)),
    ));

    println!("  t  what each host saw");
    for _ in 0..400 {
        sim.cycle();
    }
    // The two write bursts, as the memory was given them. Each is
    // whole and in order, which is what the lock on the write data
    // channel is for.
    let mut bursts = seen.borrow().clone();
    bursts.sort_by_key(|(at, _)| *at);
    for (at, beats) in &bursts {
        println!("     the memory was written {at:#06x}: {beats:02x?}");
    }
    assert_eq!(bursts.len(), 2, "two bursts arrived");
    assert_eq!(
        bursts[0],
        (0x1000, vec![0xa0, 0xa1, 0xa2, 0xa3]),
        "the first host's beats, whole and in order"
    );
    assert_eq!(
        bursts[1],
        (0x2000, vec![0xb0, 0xb1, 0xb2, 0xb3]),
        "the second host's beats, whole and in order"
    );
    stop();
    let net = Arb::lowered("axi_arbiter");
    txhdl::netlist::write_netlists_from_env(&[&net]);
    print!("\n{}", net.verilog());
}
