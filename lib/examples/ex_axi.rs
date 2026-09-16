// SPDX-License-Identifier: Apache-2.0
//! An AXI4 link, peer to peer. A host client issues three bursts
//! without waiting for any of them, and a peripheral client answers
//! them in the order its work finished, which is not the order they
//! arrived. Neither client names a channel, counts a beat, watches
//! `last` or writes an identifier: the host issues and awaits, the
//! peripheral accepts a whole decoded transaction and answers it.
//!
//! The two units between them, `AxiHost` and `AxiPer`, are the
//! hardware: they are lowered here and the build simulates each
//! netlist against this run under nvc and under Verilator. Between
//! them are the five AXI channels, `aw`, `ar`, `w`, `b` and `r`,
//! which is where a router will go.
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use txhdl::comp::trace::{stop, Wave};
use txhdl::comp::{join2, now, DefaultClock, Running, Unit};
use txhdl::pipeline::cycles;
use txhdl::types::U;
use txhdl_parts::bus::axi::{
    axi, serve, AxiHost, AxiPer, Link, Rd, Resp, Wr, Xact,
};

/// The link of this example: sixteen-bit addresses, thirty-two-bit
/// words, four lanes, two-bit identifiers, four of them. `AxiHost`
/// and `AxiPer` are units, so the widths are their type parameters.
type HostUnit = AxiHost<16, 32, 4, 2, 4>;
type PerUnit = AxiPer<16, 32, 4, 2>;
/// One accepted transaction of that link.
type Job = Xact<16, 32, 4, 2>;

/// How long the peripheral's worker takes over a transaction, by
/// address. The three bursts of the run take three different times,
/// which is what puts the answers out of order.
fn work_cycles(addr: u128) -> usize {
    match addr {
        0x10 => 14,
        0x20 => 1,
        _ => 2,
    }
}

fn main() {
    let Link {
        host,
        per,
        host_in,
        host_out,
        per_in,
        per_out,
    } = axi::<16, 32, 4, 2, 4>();
    let mut host_unit = HostUnit::default();
    let mut per_unit = PerUnit::default();
    if let Some(mut w) = Wave::from_env() {
        w.clock::<DefaultClock>();
        // The transaction level, on the host's side.
        w.add("issue", &host_in.0);
        w.add("wbeat", &host_in.1);
        w.add("grant", &host_out.3);
        w.add("done", &host_out.4);
        w.add("rdata", &host_out.5);
        w.add("release", &host_in.4);
        // The five AXI channels, traced once, at whichever end.
        w.add("aw", &host_out.0);
        w.add("ar", &host_out.1);
        w.add("w", &host_out.2);
        w.add("b", &host_in.2);
        w.add("r", &host_in.3);
        // The transaction level, on the peripheral's side.
        w.add("req", &per_out.0);
        w.add("wd", &per_out.1);
        w.add("ans", &per_in.3);
        w.add("rb", &per_in.4);
        w.add("host", &host_unit);
        w.add("per", &per_unit);
        w.start();
    }

    // What the peripheral answers reads from, and what the run must
    // agree with.
    let mem: Rc<RefCell<HashMap<u128, u128>>> = Rc::new(RefCell::new(
        (0..0x40u128).map(|a| (a, 0x1000 + a * 0x11)).collect(),
    ));

    // The host client. Three bursts are issued one after another and
    // none is awaited until all three are out, so all three are in
    // flight; each took an identifier of its own, which the client
    // never wrote.
    let answers = Rc::new(RefCell::new(Vec::new()));
    let got = answers.clone();
    let client = async move {
        let r0 = host.read(Rd::at(0x10u32, 2)).await;
        println!(
            "t={:>3}  issue read  id {} 2 words at 0x10",
            now() / 2,
            r0.id().raw()
        );
        let r1 = host.read(Rd::at(0x20u32, 1)).await;
        println!(
            "t={:>3}  issue read  id {} 1 word  at 0x20",
            now() / 2,
            r1.id().raw()
        );
        let w0 = host
            .write(Wr::at(0x30u32), &[U::from(0xaaaau32), U::from(0xbbbbu32)])
            .await;
        println!(
            "t={:>3}  issue write id {} 2 words at 0x30",
            now() / 2,
            w0.id().raw()
        );
        // Awaited in the order they were issued; the answers arrive
        // in another, and each burst still gets its own.
        let a0 = r0.done().await;
        println!(
            "t={:>3}  read   id 0 done {:?} {:x?}",
            now() / 2,
            a0.resp,
            a0.data.iter().map(|d| d.raw()).collect::<Vec<_>>()
        );
        let a1 = r1.done().await;
        println!(
            "t={:>3}  read   id 1 done {:?} {:x?}",
            now() / 2,
            a1.resp,
            a1.data.iter().map(|d| d.raw()).collect::<Vec<_>>()
        );
        let a2 = w0.done().await;
        println!("t={:>3}  write  id 2 done {:?}", now() / 2, a2.resp);
        got.borrow_mut().push((a0, a1, a2));
    };

    // The peripheral client. Three slots, so three transactions may
    // be open at once; each is answered when its own work finished,
    // which is what puts the answers out of order.
    let order = Rc::new(RefCell::new(Vec::new()));
    let finished = order.clone();
    let store = mem.clone();
    let server = serve(per, 3, move |x: Job| {
        let finished = finished.clone();
        let store = store.clone();
        async move {
            let addr = x.addr().raw();
            println!(
                "t={:>3}  accept {} id {} at {:#x}",
                now() / 2,
                if matches!(x, Xact::Read(_)) {
                    "read "
                } else {
                    "write"
                },
                x.id().raw(),
                addr
            );
            cycles(work_cycles(addr)).await;
            finished.borrow_mut().push(x.id().raw());
            match x {
                Xact::Write(w) => {
                    for (i, d) in w.data().iter().enumerate() {
                        store.borrow_mut().insert(addr + i as u128, d.raw());
                    }
                    w.ok().await;
                }
                Xact::Read(r) => {
                    let words: Vec<U<32>> = (0..r.words())
                        .map(|i| {
                            let m = store.borrow();
                            U::from(*m.get(&(addr + i as u128)).unwrap_or(&0)
                                as u32)
                        })
                        .collect();
                    r.data(&words).await;
                }
            }
        }
    });

    println!("  t  what");
    let mut sim = Running::new(join2(
        join2(
            host_unit.run(host_in, host_out),
            per_unit.run(per_in, per_out),
        ),
        join2(client, server),
    ));
    for _ in 0..70 {
        sim.cycle();
    }
    stop();

    // What the run must have done.
    let answers = answers.borrow();
    assert_eq!(answers.len(), 1, "the three bursts were not all answered");
    let (a0, a1, a2) = &answers[0];
    let raw = |r: &txhdl_parts::bus::axi::Reply<32>| -> Vec<u128> {
        r.data.iter().map(|d| d.raw()).collect()
    };
    assert_eq!(raw(a0), vec![0x1000 + 0x10 * 0x11, 0x1000 + 0x11 * 0x11]);
    assert_eq!(raw(a1), vec![0x1000 + 0x20 * 0x11]);
    assert_eq!(a2.resp, Resp::Okay);
    assert_eq!(mem.borrow()[&0x30], 0xaaaa, "the write did not land");
    assert_eq!(mem.borrow()[&0x31], 0xbbbb, "the write did not land");
    let order = order.borrow();
    println!(
        "\nanswered in the order {order:?}, issued in the order [0, 1, 2]"
    );
    assert_eq!(*order, vec![1, 2, 0], "the answers were not out of order");

    // The two units, lowered: the netlists the build simulates
    // against this run, and the Verilog the document shows.
    let host_net = HostUnit::lowered("axi_host");
    let per_net = PerUnit::lowered("axi_per");
    txhdl::netlist::write_netlists_from_env(&[&host_net, &per_net]);
    print!("\n{}\n{}", host_net.verilog(), per_net.verilog());
}
