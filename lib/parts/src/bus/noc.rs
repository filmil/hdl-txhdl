// SPDX-License-Identifier: Apache-2.0
//! A network on chip: nodes on a two-dimensional lattice, and the
//! bridges that put an AXI link across it.
//!
//! A node has a two-way link to each of its four neighbours and a
//! fifth, the exit, which is where something that is not a node
//! attaches. Every link is two virtual channels, one for requests and
//! one for responses, which is what keeps a full request path from
//! blocking the answers that would empty it.
//!
//! - [`pkt`]: what a link carries.
//! - [`switch`]: the five-port switch, one per virtual channel.
//! - [`node`]: a node, which is a switch per virtual channel.
//! - [`bridge`]: the exit, AXI on one side and packets on the other.
//!
//! The routing is dimension order, X and then Y, which on a lattice
//! is deadlock free without anything else being said: a packet moves
//! east or west until it is in its destination's column, then north
//! or south until it is at its node, then out the exit.
pub mod bridge;
pub mod mesh;
pub mod node;
pub mod pkt;
pub mod switch;

/// The network end to end: a host at one corner of a two by two
/// lattice and a memory at the far one, so that a burst crosses in X
/// and then in Y and its answer comes back the other way. Nothing on
/// either end knows the network is there; the host issues and awaits
/// as it would on a link, and the memory answers as it would.
#[cfg(test)]
mod tests {
    use super::bridge::{HostBridge, PerBridge};
    use super::mesh::lattice;
    use super::node::Node;
    use crate::bus::axi::sim::Ram;
    use crate::bus::axi::{axi, AxiHost, AxiPer, Link, Rd, Resp, Wr};
    use std::cell::RefCell;
    use std::rc::Rc;
    use txhdl::comp::{join2, join_all, Running, Unit};
    use txhdl::types::U;

    const XB: usize = 2;
    const YB: usize = 2;
    const A: usize = 16;
    const D: usize = 32;
    const S: usize = 4;
    const I: usize = 2;
    const NIDS: usize = 4;

    /// The map every host bridge uses here: everything goes to the
    /// node at 1, 1, which is where the memory is. The last entry is
    /// the default route, so its mask is zero.
    type Bridge = HostBridge<
        0,
        0,
        XB,
        YB,
        A,
        D,
        S,
        I,
        0x1000,
        0xf000,
        1,
        1,
        0x2000,
        0xf000,
        1,
        1,
        0,
        0,
        1,
        1,
    >;
    type Peri = PerBridge<1, 1, XB, YB, A, D, S, I, NIDS>;

    #[test]
    fn a_burst_crosses_the_lattice_and_its_answer_comes_back() {
        let mut net = lattice::<XB, YB, A, D, S, I>(2, 2);
        // The four nodes, row major: (0,0), (1,0), (0,1), (1,1).
        let mut n00 = Node::<0, 0, XB, YB, A, D, S, I>::default();
        let mut n10 = Node::<1, 0, XB, YB, A, D, S, I>::default();
        let mut n01 = Node::<0, 1, XB, YB, A, D, S, I>::default();
        let mut n11 = Node::<1, 1, XB, YB, A, D, S, I>::default();

        // The host's corner: its link, its tracker and the bridge.
        let Link {
            host,
            host_in,
            host_out,
            per_in: hp_in,
            per_out: hp_out,
            ..
        } = axi::<A, D, S, I, NIDS>();
        let mut htrk = AxiHost::<A, D, S, I, NIDS>::default();
        let mut hbr = Bridge::default();
        let hx = net.exits.remove(0);

        // The memory's corner: its link, its tracker and the bridge.
        let Link {
            per,
            per_in: pp_in,
            per_out: pp_out,
            host_in: ph_in,
            host_out: ph_out,
            ..
        } = axi::<A, D, S, I, NIDS>();
        let mut ptrk = AxiPer::<A, D, S, I>::default();
        let mut pbr = Peri::default();
        let px = net.exits.pop().unwrap();
        // Big enough to hold the word 0x1010 names, which is 1028.
        let ram = Ram::<A, D, S, I>::new(2048);

        let got = Rc::new(RefCell::new(Vec::new()));
        let out = got.clone();
        let client = async move {
            let w =
                host.write(Wr::at(0x1010u32), &[U::from(0xc0ffeeu32)]).await;
            let wr = w.done().await;
            let r = host.read(Rd::at(0x1010u32, 1)).await;
            let rd = r.done().await;
            out.borrow_mut().push((wr.resp, rd.resp, rd.data[0].raw()));
        };

        let nodes = join_all(vec![
            Box::pin(n00.run(net.ins.remove(0), net.outs.remove(0)))
                as std::pin::Pin<Box<dyn std::future::Future<Output = ()>>>,
            Box::pin(n10.run(net.ins.remove(0), net.outs.remove(0))),
            Box::pin(n01.run(net.ins.remove(0), net.outs.remove(0))),
            Box::pin(n11.run(net.ins.remove(0), net.outs.remove(0))),
        ]);
        let mut sim = Running::new(join2(
            join2(
                nodes,
                join2(
                    htrk.run(host_in, host_out),
                    hbr.run(
                        (hp_in.0, hp_in.1, hp_in.2, hx.p_out),
                        (hx.q_in, hp_out.2, hp_out.3),
                    ),
                ),
            ),
            join2(
                join2(
                    ptrk.run(pp_in, pp_out),
                    pbr.run(
                        (px.q_out, ph_in.2, ph_in.3),
                        (ph_out.0, ph_out.1, ph_out.2, px.p_in),
                    ),
                ),
                join2(client, ram.clone().serve(per, 2)),
            ),
        ));
        for _ in 0..600 {
            sim.cycle();
        }
        let got = got.borrow();
        assert_eq!(got.len(), 1, "the bursts never came back");
        assert_eq!(got[0].0, Resp::Okay, "the write");
        assert_eq!(got[0].1, Resp::Okay, "the read");
        assert_eq!(got[0].2, 0xc0ffee, "the read read what the write wrote");
        assert_eq!(
            ram.word(0x1010 / 4).raw(),
            0xc0ffee,
            "the word in the memory"
        );
    }

    /// Two hosts, at two corners, on one memory at a third. Each
    /// writes its own word and reads it back, and the answers have to
    /// find their way home: nothing keeps a table of who asked, so
    /// this is the packet's source stamp being right or the test
    /// failing.
    #[test]
    fn two_hosts_share_a_memory_and_each_answer_goes_home() {
        let mut net = lattice::<XB, YB, A, D, S, I>(2, 2);
        let mut n00 = Node::<0, 0, XB, YB, A, D, S, I>::default();
        let mut n10 = Node::<1, 0, XB, YB, A, D, S, I>::default();
        let mut n01 = Node::<0, 1, XB, YB, A, D, S, I>::default();
        let mut n11 = Node::<1, 1, XB, YB, A, D, S, I>::default();
        let nodes = join_all(vec![
            Box::pin(n00.run(net.ins.remove(0), net.outs.remove(0)))
                as std::pin::Pin<Box<dyn std::future::Future<Output = ()>>>,
            Box::pin(n10.run(net.ins.remove(0), net.outs.remove(0))),
            Box::pin(n01.run(net.ins.remove(0), net.outs.remove(0))),
            Box::pin(n11.run(net.ins.remove(0), net.outs.remove(0))),
        ]);
        // The memory is at 1, 1 and the two hosts at 0, 0 and 1, 0.
        let e00 = net.exits.remove(0);
        let e10 = net.exits.remove(0);
        let e11 = net.exits.pop().unwrap();

        let ram = Ram::<A, D, S, I>::new(2048);
        let Link {
            per,
            per_in: pp_in,
            per_out: pp_out,
            host_in: ph_in,
            host_out: ph_out,
            ..
        } = axi::<A, D, S, I, NIDS>();
        let mut ptrk = AxiPer::<A, D, S, I>::default();
        let mut pbr = Peri::default();

        let got = Rc::new(RefCell::new(Vec::new()));
        // One host's corner, its tracker, its bridge and its client.
        let mut mk = |at: u32, word: u32| {
            let Link {
                host,
                host_in,
                host_out,
                per_in,
                per_out,
                ..
            } = axi::<A, D, S, I, NIDS>();
            let out = got.clone();
            let client = async move {
                let w = host.write(Wr::at(at), &[U::from(word)]).await;
                assert_eq!(w.done().await.resp, Resp::Okay, "a write");
                let r = host.read(Rd::at(at, 1)).await;
                let rd = r.done().await;
                out.borrow_mut().push((at as u128, rd.data[0].raw()));
            };
            (host_in, host_out, per_in, per_out, client)
        };
        let (ain, aout, apin, apout, aclient) = mk(0x1010, 0xaaaa_1111);
        let (bin, bout, bpin, bpout, bclient) = mk(0x1020, 0xbbbb_2222);
        let mut atrk = AxiHost::<A, D, S, I, NIDS>::default();
        let mut btrk = AxiHost::<A, D, S, I, NIDS>::default();
        let mut abr = Bridge::default();
        let mut bbr = HostBridge::<
            1,
            0,
            XB,
            YB,
            A,
            D,
            S,
            I,
            0x1000,
            0xf000,
            1,
            1,
            0x2000,
            0xf000,
            1,
            1,
            0,
            0,
            1,
            1,
        >::default();

        let hosts = join2(
            join2(
                atrk.run(ain, aout),
                abr.run(
                    (apin.0, apin.1, apin.2, e00.p_out),
                    (e00.q_in, apout.2, apout.3),
                ),
            ),
            join2(
                btrk.run(bin, bout),
                bbr.run(
                    (bpin.0, bpin.1, bpin.2, e10.p_out),
                    (e10.q_in, bpout.2, bpout.3),
                ),
            ),
        );
        let memory = join2(
            ptrk.run(pp_in, pp_out),
            pbr.run(
                (e11.q_out, ph_in.2, ph_in.3),
                (ph_out.0, ph_out.1, ph_out.2, e11.p_in),
            ),
        );
        let mut sim = Running::new(join2(
            join2(nodes, hosts),
            join2(
                join2(memory, ram.clone().serve(per, 4)),
                join2(aclient, bclient),
            ),
        ));
        for _ in 0..1200 {
            sim.cycle();
        }
        let mut got = got.borrow().clone();
        got.sort();
        assert_eq!(got.len(), 2, "both hosts did not finish");
        assert_eq!(
            got,
            vec![(0x1010u128, 0xaaaa_1111u128), (0x1020, 0xbbbb_2222)],
            "an answer went to the wrong host"
        );
        assert_eq!(ram.word(0x1010 / 4).raw(), 0xaaaa_1111);
        assert_eq!(ram.word(0x1020 / 4).raw(), 0xbbbb_2222);
    }
}
