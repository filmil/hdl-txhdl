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
    use crate::bus::axi::{
        axi, Ar, Aw, AxiHost, AxiPer, BurstKind, Link, Rd, Resp, Wr, B, R, W,
    };
    use crate::bus::router::Router;
    use std::cell::RefCell;
    use std::rc::Rc;
    use txhdl::comp::{chan, join2, join_all, DefaultClock, Running, Unit};
    use txhdl::map::AddrMap;
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
        let mut n00 = Node::<XB, YB, A, D, S, I>::default();
        let mut n10 = Node::<XB, YB, A, D, S, I>::default();
        let mut n01 = Node::<XB, YB, A, D, S, I>::default();
        let mut n11 = Node::<XB, YB, A, D, S, I>::default();

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

    /// A read of four beats across the network. A write of more than
    /// one beat is refused (issue 125), because the beats after the
    /// first carry no address and would be paired with the next
    /// burst's address phase. A read is the other way round: the
    /// network carries one request packet, and every beat of the
    /// answer is a packet of its own with the identifier and `last`
    /// in it, so nothing has to be remembered between them. Issue 133
    /// asks whether that is true in fact as well as in argument, and
    /// this is the answer: the four words come back, in order, with
    /// the right data, and the last is marked.
    #[test]
    fn a_read_of_four_beats_crosses_the_lattice() {
        let mut net = lattice::<XB, YB, A, D, S, I>(2, 2);
        let mut n00 = Node::<XB, YB, A, D, S, I>::default();
        let mut n10 = Node::<XB, YB, A, D, S, I>::default();
        let mut n01 = Node::<XB, YB, A, D, S, I>::default();
        let mut n11 = Node::<XB, YB, A, D, S, I>::default();

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
        let ram = Ram::<A, D, S, I>::new(2048);

        let got = Rc::new(RefCell::new(Vec::new()));
        let out = got.clone();
        let client = async move {
            // Four words, one beat each, since a write of several
            // beats is refused; then one read of four beats.
            for i in 0..4u32 {
                let w = host
                    .write(Wr::at(0x1000u32 + 4 * i), &[U::from(0xa0 + i)])
                    .await;
                w.done().await;
            }
            let r = host.read(Rd::at(0x1000u32, 4)).await;
            let rd = r.done().await;
            out.borrow_mut().push((
                rd.resp,
                rd.data.iter().map(|w| w.raw()).collect::<Vec<_>>(),
            ));
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
        for _ in 0..2000 {
            sim.cycle();
        }
        let got = got.borrow();
        assert_eq!(got.len(), 1, "the read never came back");
        assert_eq!(got[0].0, Resp::Okay, "the read");
        assert_eq!(
            got[0].1,
            vec![0xa0, 0xa1, 0xa2, 0xa3],
            "four beats, in order, each with its own word"
        );
    }

    /// A write of two beats across the network: the host bridge splits
    /// it into two single-beat writes at consecutive addresses (issue
    /// 125), the memory takes both, and the host is answered once,
    /// `Okay`. Before the split such a burst was refused, and before
    /// the refusal this test hung: the write left as one packet marked
    /// last, the peripheral's tracker waited for a beat that had
    /// stayed behind, and the client waited for ever.
    #[test]
    fn a_write_of_two_beats_crosses_the_lattice_as_two_writes() {
        let mut net = lattice::<XB, YB, A, D, S, I>(2, 2);
        let mut n00 = Node::<XB, YB, A, D, S, I>::default();
        let mut n10 = Node::<XB, YB, A, D, S, I>::default();
        let mut n01 = Node::<XB, YB, A, D, S, I>::default();
        let mut n11 = Node::<XB, YB, A, D, S, I>::default();

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
        let ram = Ram::<A, D, S, I>::new(2048);

        let got = Rc::new(RefCell::new(Vec::new()));
        let out = got.clone();
        let client = async move {
            let w = host
                .write(
                    Wr::at(0x1010u32),
                    &[U::from(0xaaaaaau32), U::from(0xbbbbbbu32)],
                )
                .await;
            let wr = w.done().await;
            out.borrow_mut().push(wr.resp);
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
        assert_eq!(got.len(), 1, "the write never answered");
        assert_eq!(got[0], Resp::Okay, "a long burst is carried");
        assert_eq!(ram.word(0x1010 / 4).raw(), 0xaaaaaa, "the first beat");
        assert_eq!(
            ram.word(0x1014 / 4).raw(),
            0xbbbbbb,
            "the second, after it"
        );
    }

    /// A wrapping burst is the one still refused, since the wrap is
    /// not computed at the bridge. The refusal is of that burst and not
    /// of the bridge: a write of one beat sent after it still crosses
    /// and is answered, so the eaten beats left nothing behind on the
    /// channel.
    #[test]
    fn a_wrapping_write_is_refused_and_the_one_after_it_is_served() {
        let mut net = lattice::<XB, YB, A, D, S, I>(2, 2);
        let mut n00 = Node::<XB, YB, A, D, S, I>::default();
        let mut n10 = Node::<XB, YB, A, D, S, I>::default();
        let mut n01 = Node::<XB, YB, A, D, S, I>::default();
        let mut n11 = Node::<XB, YB, A, D, S, I>::default();

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
        let ram = Ram::<A, D, S, I>::new(2048);

        let got = Rc::new(RefCell::new(Vec::new()));
        let out = got.clone();
        let client = async move {
            let bad = host
                .write(
                    Wr {
                        burst: BurstKind::Wrap,
                        ..Wr::at(0x1010u32)
                    },
                    &[U::from(0xaaaaaau32), U::from(0xbbbbbbu32)],
                )
                .await;
            let first = bad.done().await;
            let good =
                host.write(Wr::at(0x1020u32), &[U::from(0xc0ffeeu32)]).await;
            let second = good.done().await;
            out.borrow_mut().push((first.resp, second.resp));
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
        for _ in 0..900 {
            sim.cycle();
        }
        let got = got.borrow();
        assert_eq!(got.len(), 1, "the writes never came back");
        assert_eq!(got[0].0, Resp::SlvErr, "the wrapping burst is refused");
        assert_eq!(got[0].1, Resp::Okay, "the one after it is served");
        assert_eq!(
            ram.word(0x1020 / 4).raw(),
            0xc0ffee,
            "and its word reached the memory"
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
        let mut n00 = Node::<XB, YB, A, D, S, I>::default();
        let mut n10 = Node::<XB, YB, A, D, S, I>::default();
        let mut n01 = Node::<XB, YB, A, D, S, I>::default();
        let mut n11 = Node::<XB, YB, A, D, S, I>::default();
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
        let mk = |at: u32, word: u32| {
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

    /// The word host `h` writes into peripheral `k`.
    fn word(h: u32, k: usize) -> u32 {
        0x5eed_0000 | (h << 8) | k as u32
    }

    /// Four peripherals behind one node: a router of four behind the
    /// peripheral bridge at 1, 1, a memory in each of its ranges, and
    /// a host at each of two other corners. Each host writes a word
    /// into every peripheral, reads each back, and then reads an
    /// address the network sends to the node but no peripheral has.
    /// The network sees one exit and the router decodes behind it, so
    /// a word in the wrong memory, an answer at the wrong host, or a
    /// hole nobody answers fails the test.
    #[test]
    fn four_peripherals_share_one_node() {
        type HostAt<const X: usize, const Y: usize> = HostBridge<
            X,
            Y,
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
        // A quarter of 0x1000..0x2000 each. 0x2000 reaches the node
        // by the hosts' map and is none of the router's.
        struct NocMap;

        impl AddrMap<4> for NocMap {
            const RANGES: [(usize, usize); 4] = [
                (0x1000, 0xfc00),
                (0x1400, 0xfc00),
                (0x1800, 0xfc00),
                (0x1c00, 0xfc00),
            ];
        }

        type Rtr = Router<4, NocMap, A, D, S, I>;
        const BASES: [u32; 4] = [0x1000, 0x1400, 0x1800, 0x1c00];
        const HOLE: u32 = 0x2010;
        type Boxed<'a> =
            std::pin::Pin<Box<dyn std::future::Future<Output = ()> + 'a>>;

        let mut net = lattice::<XB, YB, A, D, S, I>(2, 2);
        let mut n00 = Node::<XB, YB, A, D, S, I>::default();
        let mut n10 = Node::<XB, YB, A, D, S, I>::default();
        let mut n01 = Node::<XB, YB, A, D, S, I>::default();
        let mut n11 = Node::<XB, YB, A, D, S, I>::default();
        let nodes = join_all(vec![
            Box::pin(n00.run(net.ins.remove(0), net.outs.remove(0)))
                as Boxed<'_>,
            Box::pin(n10.run(net.ins.remove(0), net.outs.remove(0))),
            Box::pin(n01.run(net.ins.remove(0), net.outs.remove(0))),
            Box::pin(n11.run(net.ins.remove(0), net.outs.remove(0))),
        ]);
        // The hosts are at 0, 0 and 1, 0; the peripherals at 1, 1.
        let e00 = net.exits.remove(0);
        let e10 = net.exits.remove(0);
        let e11 = net.exits.pop().unwrap();

        // The node's one exit, its bridge, and the router behind it,
        // on the five AXI channels between the two.
        let mut pbr = Peri::default();
        let mut rtr = Rtr::default();
        let (aw_tx, aw_rx) = chan::<Aw<A, I>, DefaultClock>();
        let (ar_tx, ar_rx) = chan::<Ar<A, I>, DefaultClock>();
        let (w_tx, w_rx) = chan::<W<D, S>, DefaultClock>();
        let (b_tx, b_rx) = chan::<B<I>, DefaultClock>();
        let (r_tx, r_rx) = chan::<R<D, I>, DefaultClock>();

        // The four peripherals: each a link's peripheral half, its
        // tracker, and a memory that serves it.
        let rams: Vec<Ram<A, D, S, I>> =
            (0..4).map(|_| Ram::new(2048)).collect();
        let mut t0 = AxiPer::<A, D, S, I>::default();
        let mut t1 = AxiPer::<A, D, S, I>::default();
        let mut t2 = AxiPer::<A, D, S, I>::default();
        let mut t3 = AxiPer::<A, D, S, I>::default();
        let Link {
            per: p0,
            per_in: i0,
            per_out: o0,
            host_in: h0,
            host_out: g0,
            ..
        } = axi::<A, D, S, I, NIDS>();
        let Link {
            per: p1,
            per_in: i1,
            per_out: o1,
            host_in: h1,
            host_out: g1,
            ..
        } = axi::<A, D, S, I, NIDS>();
        let Link {
            per: p2,
            per_in: i2,
            per_out: o2,
            host_in: h2,
            host_out: g2,
            ..
        } = axi::<A, D, S, I, NIDS>();
        let Link {
            per: p3,
            per_in: i3,
            per_out: o3,
            host_in: h3,
            host_out: g3,
            ..
        } = axi::<A, D, S, I, NIDS>();
        let peripherals =
            join_all(vec![
                Box::pin(pbr.run(
                    (e11.q_out, b_rx, r_rx),
                    (aw_tx, ar_tx, w_tx, e11.p_in),
                )) as Boxed<'_>,
                Box::pin(rtr.run(
                    (
                        aw_rx,
                        ar_rx,
                        w_rx,
                        [h0.2, h1.2, h2.2, h3.2],
                        [h0.3, h1.3, h2.3, h3.3],
                    ),
                    (
                        [g0.0, g1.0, g2.0, g3.0],
                        [g0.1, g1.1, g2.1, g3.1],
                        [g0.2, g1.2, g2.2, g3.2],
                        b_tx,
                        r_tx,
                    ),
                )),
                Box::pin(t0.run(i0, o0)),
                Box::pin(t1.run(i1, o1)),
                Box::pin(t2.run(i2, o2)),
                Box::pin(t3.run(i3, o3)),
                Box::pin(rams[0].clone().serve(p0, 2)),
                Box::pin(rams[1].clone().serve(p1, 2)),
                Box::pin(rams[2].clone().serve(p2, 2)),
                Box::pin(rams[3].clone().serve(p3, 2)),
            ]);

        // One host's corner: its link, and a client that writes every
        // peripheral, reads every one back, and reads the hole.
        let got = Rc::new(RefCell::new(Vec::new()));
        let mk = |h: u32| {
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
                for (k, base) in BASES.iter().enumerate() {
                    let at = base + 0x10 + 4 * h;
                    let w =
                        host.write(Wr::at(at), &[U::from(word(h, k))]).await;
                    assert_eq!(w.done().await.resp, Resp::Okay, "a write");
                }
                let mut seen = Vec::new();
                for base in BASES {
                    let at = base + 0x10 + 4 * h;
                    let rd = host.read(Rd::at(at, 1)).await.done().await;
                    seen.push((rd.resp, rd.data[0].raw()));
                }
                let hole = host.read(Rd::at(HOLE, 1)).await.done().await;
                seen.push((hole.resp, 0));
                out.borrow_mut().push((h, seen));
            };
            (host_in, host_out, per_in, per_out, client)
        };
        let (ain, aout, apin, apout, aclient) = mk(0);
        let (bin, bout, bpin, bpout, bclient) = mk(1);
        let mut atrk = AxiHost::<A, D, S, I, NIDS>::default();
        let mut btrk = AxiHost::<A, D, S, I, NIDS>::default();
        let mut abr = HostAt::<0, 0>::default();
        let mut bbr = HostAt::<1, 0>::default();
        let hosts = join_all(vec![
            Box::pin(atrk.run(ain, aout)) as Boxed<'_>,
            Box::pin(abr.run(
                (apin.0, apin.1, apin.2, e00.p_out),
                (e00.q_in, apout.2, apout.3),
            )),
            Box::pin(btrk.run(bin, bout)),
            Box::pin(bbr.run(
                (bpin.0, bpin.1, bpin.2, e10.p_out),
                (e10.q_in, bpout.2, bpout.3),
            )),
        ]);

        let mut sim = Running::new(join2(
            join2(nodes, hosts),
            join2(peripherals, join2(aclient, bclient)),
        ));
        for _ in 0..4000 {
            sim.cycle();
        }
        let mut got = got.borrow().clone();
        got.sort_by_key(|(h, _)| *h);
        assert_eq!(got.len(), 2, "both hosts did not finish");
        for (h, seen) in &got {
            for (k, answer) in seen[..4].iter().enumerate() {
                assert_eq!(
                    *answer,
                    (Resp::Okay, word(*h, k) as u128),
                    "host {h} read peripheral {k}"
                );
            }
            assert_eq!(seen[4].0, Resp::DecErr, "host {h} read the hole");
        }
        // Each word is in its own peripheral's memory and in no other.
        for h in 0..2 {
            for (k, base) in BASES.iter().enumerate() {
                let at = ((base + 0x10 + 4 * h) / 4) as usize;
                for (j, ram) in rams.iter().enumerate() {
                    let want = if j == k { word(h, k) } else { 0 };
                    assert_eq!(
                        ram.word(at).raw(),
                        want as u128,
                        "host {h}'s word for peripheral {k}, in memory {j}"
                    );
                }
            }
        }
    }

    /// One switch, with its north input never empty and its exit
    /// asking for the same output. The two take turns, so a node can
    /// always put its own traffic into a network whose links are
    /// busy.
    ///
    /// This was the arbitration question in issue 133 and then the
    /// defect in issue 315. Under the fixed order it answered, the
    /// link moved 198 packets in two hundred cycles and the exit
    /// moved none, for as long as the link kept offering and with no
    /// bound on the wait. Under the round robin they alternate.
    #[test]
    fn a_busy_link_and_the_exit_take_turns() {
        use super::pkt::{Chan, Pkt};
        use super::switch::Switch;
        use txhdl::types::Bit;
        let mut sw = Switch::<XB, YB, A, D, S, I>::default();
        // Five inputs and five outputs, as the switch takes them.
        let (n_tx, n_rx) = chan::<Pkt<XB, YB, A, D, S, I>, DefaultClock>();
        let (s_tx, s_rx) = chan::<Pkt<XB, YB, A, D, S, I>, DefaultClock>();
        let (w_tx, w_rx) = chan::<Pkt<XB, YB, A, D, S, I>, DefaultClock>();
        let (e_tx, e_rx) = chan::<Pkt<XB, YB, A, D, S, I>, DefaultClock>();
        let (x_tx, x_rx) = chan::<Pkt<XB, YB, A, D, S, I>, DefaultClock>();
        let (no_tx, no_rx) = chan::<Pkt<XB, YB, A, D, S, I>, DefaultClock>();
        let (so_tx, so_rx) = chan::<Pkt<XB, YB, A, D, S, I>, DefaultClock>();
        let (wo_tx, wo_rx) = chan::<Pkt<XB, YB, A, D, S, I>, DefaultClock>();
        let (eo_tx, eo_rx) = chan::<Pkt<XB, YB, A, D, S, I>, DefaultClock>();
        let (xo_tx, xo_rx) = chan::<Pkt<XB, YB, A, D, S, I>, DefaultClock>();
        // A packet for the node one column east, which leaves by the
        // east port whichever input it arrived on. `id` says who sent
        // it: 1 from the link, 2 from the exit.
        let pkt = |id: u32| Pkt::<XB, YB, A, D, S, I> {
            dx: U::from(1u32),
            dy: U::from(0u32),
            sx: U::from(0u32),
            sy: U::from(0u32),
            chan: Chan::Ar,
            id: U::from(id),
            last: Bit::One,
            ..Pkt::default()
        };
        let _ = (s_tx, w_tx, e_tx);
        let mut sim = Running::new(sw.run(
            (
                txhdl::comp::tie(U::<XB>::from(0u8)),
                txhdl::comp::tie(U::<YB>::from(0u8)),
                n_rx,
                s_rx,
                w_rx,
                e_rx,
                x_rx,
            ),
            (no_tx, so_tx, wo_tx, eo_tx, xo_tx),
        ));
        let mut seen: Vec<u32> = Vec::new();
        for _ in 0..200 {
            // The link and the exit both offer, every cycle, for as
            // long as there is room to offer.
            if n_tx.ready().to_bool() {
                n_tx.send(pkt(1));
            }
            if x_tx.ready().to_bool() {
                x_tx.send(pkt(2));
            }
            // Whatever leaves by the east port is counted by who sent
            // it, and the four ports nothing uses are drained so that
            // the switch is never held up by one of them.
            if let Some(p) = eo_rx.recv() {
                seen.push(p.id.raw() as u32);
            }
            let _ = no_rx.recv();
            let _ = so_rx.recv();
            let _ = wo_rx.recv();
            let _ = xo_rx.recv();
            sim.cycle();
        }
        let from_the_link = seen.iter().filter(|id| **id == 1).count();
        let from_the_exit = seen.iter().filter(|id| **id == 2).count();
        assert!(from_the_link > 50, "the link moved {from_the_link}");
        assert!(
            from_the_exit > 50,
            "the exit moved {from_the_exit} packets against the link's \
             {from_the_link} in 200 cycles: it is being starved"
        );
        // Neither had to wait on the other for more than its turn.
        let gap = from_the_link.abs_diff(from_the_exit);
        assert!(gap <= 2, "{from_the_link} against {from_the_exit}");
    }

    /// Two inputs of one switch leaving by the same port, in the two
    /// regimes that matter. Both offering every cycle: they alternate,
    /// one each. One offering every third cycle: the other takes the
    /// port the rest of the time, and the two mix at the output.
    ///
    /// That is blocker 2 of issue 133, measured rather than argued.
    /// The blocker says beats from different sources interleave
    /// wherever their paths merge, and they do; under the fixed order
    /// this replaced, the lower source was starved instead while the
    /// higher never paused, which is issue 315. Either way a burst's
    /// beats do not stay together, so whatever carries a multi-beat
    /// write has to keep them together itself at every switch they
    /// pass, and the round robin makes that a certainty rather than a
    /// matter of who is busy.
    #[test]
    fn two_inputs_of_a_switch_contend_for_their_shared_output() {
        use super::pkt::{Chan, Pkt};
        use super::switch::Switch;
        use txhdl::types::Bit;
        let mut sw = Switch::<XB, YB, A, D, S, I>::default();
        let (n_tx, n_rx) = chan::<Pkt<XB, YB, A, D, S, I>, DefaultClock>();
        let (s_tx, s_rx) = chan::<Pkt<XB, YB, A, D, S, I>, DefaultClock>();
        let (w_tx, w_rx) = chan::<Pkt<XB, YB, A, D, S, I>, DefaultClock>();
        let (e_tx, e_rx) = chan::<Pkt<XB, YB, A, D, S, I>, DefaultClock>();
        let (x_tx, x_rx) = chan::<Pkt<XB, YB, A, D, S, I>, DefaultClock>();
        let (no_tx, no_rx) = chan::<Pkt<XB, YB, A, D, S, I>, DefaultClock>();
        let (so_tx, so_rx) = chan::<Pkt<XB, YB, A, D, S, I>, DefaultClock>();
        let (wo_tx, wo_rx) = chan::<Pkt<XB, YB, A, D, S, I>, DefaultClock>();
        let (eo_tx, eo_rx) = chan::<Pkt<XB, YB, A, D, S, I>, DefaultClock>();
        let (xo_tx, xo_rx) = chan::<Pkt<XB, YB, A, D, S, I>, DefaultClock>();
        // Both sources send to the node one column east, so both
        // leave by the east port. `id` says which source.
        let pkt = |id: u32| Pkt::<XB, YB, A, D, S, I> {
            dx: U::from(1u32),
            dy: U::from(0u32),
            chan: Chan::W,
            id: U::from(id),
            last: Bit::One,
            ..Pkt::default()
        };
        let _ = (w_tx, e_tx, x_tx);
        let mut sim = Running::new(sw.run(
            (
                txhdl::comp::tie(U::<XB>::from(0u8)),
                txhdl::comp::tie(U::<YB>::from(0u8)),
                n_rx,
                s_rx,
                w_rx,
                e_rx,
                x_rx,
            ),
            (no_tx, so_tx, wo_tx, eo_tx, xo_tx),
        ));
        // Forty cycles with both offering always, then forty with the
        // higher input offering every third cycle.
        let mut saturated: Vec<u32> = Vec::new();
        let mut sharing: Vec<u32> = Vec::new();
        for c in 0..80 {
            let north = c < 40 || c % 3 == 0;
            if north && n_tx.ready().to_bool() {
                n_tx.send(pkt(1));
            }
            if s_tx.ready().to_bool() {
                s_tx.send(pkt(2));
            }
            if let Some(p) = eo_rx.recv() {
                let id = p.id.raw() as u32;
                if c < 40 {
                    saturated.push(id)
                } else {
                    sharing.push(id)
                }
            }
            let _ = no_rx.recv();
            let _ = so_rx.recv();
            let _ = wo_rx.recv();
            let _ = xo_rx.recv();
            sim.cycle();
        }
        // Saturated: they alternate, one each, and neither takes two
        // in a row.
        assert!(saturated.len() > 20, "only {} moved", saturated.len());
        let north = saturated.iter().filter(|id| **id == 1).count();
        let south = saturated.len() - north;
        assert!(
            north.abs_diff(south) <= 2,
            "one input took the port: {saturated:?}"
        );
        let runs = saturated.windows(2).filter(|w| w[0] == w[1]).count();
        assert_eq!(runs, 0, "two together {runs} times: {saturated:?}");
        // Sharing: both get through, and the higher input's packets
        // land between the lower input's.
        assert!(sharing.contains(&1) && sharing.contains(&2), "{sharing:?}");
        let cuts = sharing.windows(3).filter(|w| w[1] != w[0]).count();
        assert!(
            cuts > 3,
            "the two sources did not mix at the output: {sharing:?}"
        );
    }
    /// A fixed burst is every beat at one address, and the split
    /// keeps it so: three words to one address leave as three writes
    /// there, and the word holds the last of them, with the one after
    /// it untouched.
    #[test]
    fn a_fixed_write_lands_every_beat_on_one_word() {
        let mut net = lattice::<XB, YB, A, D, S, I>(2, 2);
        let mut n00 = Node::<XB, YB, A, D, S, I>::default();
        let mut n10 = Node::<XB, YB, A, D, S, I>::default();
        let mut n01 = Node::<XB, YB, A, D, S, I>::default();
        let mut n11 = Node::<XB, YB, A, D, S, I>::default();

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
        let ram = Ram::<A, D, S, I>::new(2048);

        let got = Rc::new(RefCell::new(Vec::new()));
        let out = got.clone();
        let client = async move {
            let w = host
                .write(
                    Wr {
                        burst: BurstKind::Fixed,
                        ..Wr::at(0x1010u32)
                    },
                    &[U::from(1u32), U::from(2u32), U::from(3u32)],
                )
                .await;
            let wr = w.done().await;
            out.borrow_mut().push(wr.resp);
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
        assert_eq!(got.len(), 1, "the write never answered");
        assert_eq!(got[0], Resp::Okay, "a fixed burst is carried");
        assert_eq!(ram.word(0x1010 / 4).raw(), 3, "the last beat stands");
        assert_eq!(ram.word(0x1014 / 4).raw(), 0, "the next word untouched");
    }

    /// Two hosts, at two corners, each writing sixteen words at once
    /// to one memory at a third, and reading them back. The two
    /// bursts are in the network together, cut into each other
    /// wherever their paths merge, and every word still lands in its
    /// place and in its order, because each beat left as a whole
    /// write of its own. This is what issue 125 asked for.
    #[test]
    fn two_hosts_write_sixteen_beats_at_once_and_every_word_lands() {
        let mut net = lattice::<XB, YB, A, D, S, I>(2, 2);
        let mut n00 = Node::<XB, YB, A, D, S, I>::default();
        let mut n10 = Node::<XB, YB, A, D, S, I>::default();
        let mut n01 = Node::<XB, YB, A, D, S, I>::default();
        let mut n11 = Node::<XB, YB, A, D, S, I>::default();
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

        const BEATS: usize = 16;
        /// The word host `h` puts at beat `k`.
        fn beat(h: u32, k: usize) -> u32 {
            (h << 24) | 0x0011_0000 | k as u32
        }
        let got = Rc::new(RefCell::new(Vec::new()));
        // One host's corner, its tracker, its bridge and its client.
        let mk = |h: u32, at: u32| {
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
                let words: Vec<U<D>> =
                    (0..BEATS).map(|k| U::from(beat(h, k))).collect();
                let w = host.write(Wr::at(at), &words).await;
                assert_eq!(w.done().await.resp, Resp::Okay, "the burst");
                let r = host.read(Rd::at(at, BEATS)).await;
                let rd = r.done().await;
                assert_eq!(rd.resp, Resp::Okay, "the read back");
                let back: Vec<u128> = rd.data.iter().map(|d| d.raw()).collect();
                out.borrow_mut().push((h, back));
            };
            (host_in, host_out, per_in, per_out, client)
        };
        let (ain, aout, apin, apout, aclient) = mk(0xa, 0x1100);
        let (bin, bout, bpin, bpout, bclient) = mk(0xb, 0x1200);
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
        for _ in 0..3000 {
            sim.cycle();
        }
        let mut got = got.borrow().clone();
        got.sort();
        assert_eq!(got.len(), 2, "both hosts did not finish");
        for (h, at) in [(0xau32, 0x1100usize), (0xb, 0x1200)] {
            let want: Vec<u128> =
                (0..BEATS).map(|k| beat(h, k) as u128).collect();
            let (_, back) = got.iter().find(|(g, _)| *g == h).unwrap();
            assert_eq!(*back, want, "host {h:x} read back its burst in order");
            for k in 0..BEATS {
                assert_eq!(
                    ram.word(at / 4 + k).raw(),
                    beat(h, k) as u128,
                    "host {h:x}, beat {k}, in the memory"
                );
            }
        }
    }
}
