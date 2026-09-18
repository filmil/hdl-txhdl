// SPDX-License-Identifier: Apache-2.0
//! The AXI arbiter: several hosts, one peripheral link. It is the
//! mirror of the [`router`](super::router), which is one host and
//! several peripherals, and the two go together: an arbiter merges
//! the hosts onto one link and a router fans that link out again.
//!
//! Until there was one, the core was the only thing on this bus that
//! could be a host. Everything else, an Ethernet MAC with a frame to
//! store or a rasteriser with a scene to draw, had to be given its
//! bytes a word at a time by the core through a register.
//!
//! The problem an arbiter has that a router does not is that the
//! answers must find their way home. AXI4 puts an identifier on each
//! address phase and carries it back on the answer, so the arbiter
//! puts the host's port number above the host's own identifier on the
//! way out and takes it off again on the way back. That is why the
//! peripheral side's identifier is `J` bits where a host's is `I`:
//! `J` has to be at least `I` plus the bits the port number needs.
//!
//! The write data channel has no identifier at all, which is the
//! other half of the same problem. A beat belongs to the oldest
//! address phase that has not finished, so the arbiter locks that
//! channel to whichever host won the write address phase until its
//! last beat. Reads need no lock, since each carries its identifier
//! and may be answered out of order.
//!
//! One count of hosts is one unit, written out by `arbiter!`, since
//! the lowering reads a body and not a loop over ports.
use txhdl::arbiter;

// begin{part}
arbiter!(Arbiter2, 2);
arbiter!(Arbiter3, 3);
arbiter!(Arbiter4, 4);
arbiter!(Arbiter5, 5);
arbiter!(Arbiter6, 6);
arbiter!(Arbiter7, 7);
arbiter!(Arbiter8, 8);
// end{part}

/// The arbiter against the rules it has to keep: every host's burst
/// reaches the peripheral, every answer comes back to the host that
/// asked, the beats of two write bursts do not interleave, and no
/// host is starved while another keeps asking.
#[cfg(test)]
mod tests {
    use super::{Arbiter2, Arbiter4};
    use crate::bus::axi::{
        axi, AxiHost, AxiPer, Host, Per, Rd, Resp, Wr, Xact,
    };
    use std::cell::RefCell;
    use std::rc::Rc;
    use txhdl::comp::{join2, Running, Unit};
    use txhdl::types::U;

    /// A host's identifier is two bits, the peripheral side's five,
    /// which leaves three for the port number: more than the two
    /// hosts need, and exactly what four need.
    type HostUnit = AxiHost<16, 32, 4, 2, 8>;
    type PerUnit = AxiPer<16, 32, 4, 5>;
    type Job = Xact<16, 32, 4, 5>;
    type Client = Host<16, 32, 4, 2, 8>;

    /// A client, boxed so that two of different shapes can be passed
    /// to one driver.
    type Boxed = Box<dyn std::future::Future<Output = ()> + Unpin>;

    /// What the peripheral was asked to write, in the order the beats
    /// arrived, and what it was asked to read.
    #[derive(Clone, Default)]
    struct Seen(Rc<RefCell<Vec<(u128, Vec<u128>)>>>);

    /// A memory that answers a read with the address it was asked
    /// for, so a host can tell its own answers from another's, and
    /// records every write burst whole.
    fn memory(
        per: Per<16, 32, 4, 5>,
        seen: Seen,
    ) -> impl std::future::Future<Output = ()> {
        crate::bus::axi::serve(per, 4, move |x: Job| {
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
                        seen.0.borrow_mut().push((wr.addr().raw(), beats));
                        wr.ok().await
                    }
                }
            }
        })
    }

    /// Two hosts, an arbiter between them and one memory. Each host's
    /// client is built by the caller; the run is `n` cycles.
    fn two(
        a: impl FnOnce(Client) -> Boxed,
        b: impl FnOnce(Client) -> Boxed,
        n: usize,
    ) -> Seen {
        let l0 = axi::<16, 32, 4, 2, 8>();
        let l1 = axi::<16, 32, 4, 2, 8>();
        let lp = axi::<16, 32, 4, 5, 8>();
        let seen = Seen::default();
        let mut h0 = HostUnit::default();
        let mut h1 = HostUnit::default();
        let mut pu = PerUnit::default();
        let mut arb = Arbiter2::<16, 32, 4, 2, 5, 0>::default();
        let hardware = join2(
            join2(
                h0.run(l0.host_in, l0.host_out),
                h1.run(l1.host_in, l1.host_out),
            ),
            join2(
                pu.run(lp.per_in, lp.per_out),
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
        );
        let clients = join2(a(l0.host), b(l1.host));
        let mut sim = Running::new(join2(
            join2(hardware, memory(lp.per, seen.clone())),
            clients,
        ));
        for _ in 0..n {
            sim.cycle();
        }
        seen
    }

    /// Two hosts read at once. Each gets its own answer back: the
    /// memory answers with the address, so a host that was given
    /// another's beats would see an address it never asked for.
    #[test]
    fn each_answer_goes_home() {
        let got0 = Rc::new(RefCell::new(Vec::new()));
        let got1 = Rc::new(RefCell::new(Vec::new()));
        let (r0, r1) = (got0.clone(), got1.clone());
        two(
            move |host| {
                Box::new(Box::pin(async move {
                    for at in [0x100u32, 0x200, 0x300] {
                        let p = host.read(Rd::at(at, 2)).await;
                        let r = p.done().await;
                        r0.borrow_mut().push((
                            r.resp,
                            r.data.iter().map(|w| w.raw()).collect::<Vec<_>>(),
                        ));
                    }
                }))
            },
            move |host| {
                Box::new(Box::pin(async move {
                    for at in [0x900u32, 0xa00, 0xb00] {
                        let p = host.read(Rd::at(at, 2)).await;
                        let r = p.done().await;
                        r1.borrow_mut().push((
                            r.resp,
                            r.data.iter().map(|w| w.raw()).collect::<Vec<_>>(),
                        ));
                    }
                }))
            },
            400,
        );
        let a = got0.borrow().clone();
        let b = got1.borrow().clone();
        assert_eq!(a.len(), 3, "the first host was answered three times");
        assert_eq!(b.len(), 3, "and so was the second");
        assert_eq!(a[0], (Resp::Okay, vec![0x100, 0x104]), "its own address");
        assert_eq!(a[1], (Resp::Okay, vec![0x200, 0x204]));
        assert_eq!(a[2], (Resp::Okay, vec![0x300, 0x304]));
        assert_eq!(b[0], (Resp::Okay, vec![0x900, 0x904]), "and its own");
        assert_eq!(b[1], (Resp::Okay, vec![0xa00, 0xa04]));
        assert_eq!(b[2], (Resp::Okay, vec![0xb00, 0xb04]));
    }

    /// Two hosts write multi-beat bursts at the same time. AXI4 puts
    /// no identifier on the write data channel, so the arbiter must
    /// hold that channel for the burst it granted; if it did not, the
    /// memory would be given a mixture of the two.
    #[test]
    fn write_beats_do_not_interleave() {
        let seen = two(
            |host| {
                Box::new(Box::pin(async move {
                    let p = host
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
                    assert_eq!(p.done().await.resp, Resp::Okay);
                }))
            },
            |host| {
                Box::new(Box::pin(async move {
                    let p = host
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
                    assert_eq!(p.done().await.resp, Resp::Okay);
                }))
            },
            400,
        );
        let mut bursts = seen.0.borrow().clone();
        bursts.sort_by_key(|(at, _)| *at);
        assert_eq!(bursts.len(), 2, "two bursts, whole");
        assert_eq!(
            bursts[0],
            (0x1000, vec![0xa0, 0xa1, 0xa2, 0xa3]),
            "the first host's beats, in order and unmixed"
        );
        assert_eq!(
            bursts[1],
            (0x2000, vec![0xb0, 0xb1, 0xb2, 0xb3]),
            "the second host's beats, in order and unmixed"
        );
    }

    /// Four hosts, all reading as fast as they can. Round robin means
    /// none of them is starved: the run is long enough for each to
    /// get several bursts through, and the counts are within one of
    /// each other.
    #[test]
    fn four_hosts_share_the_link() {
        let links: Vec<_> = (0..4).map(|_| axi::<16, 32, 4, 2, 8>()).collect();
        let lp = axi::<16, 32, 4, 5, 8>();
        let counts: Vec<Rc<RefCell<usize>>> =
            (0..4).map(|_| Rc::new(RefCell::new(0))).collect();
        let mut hs: Vec<HostUnit> =
            (0..4).map(|_| HostUnit::default()).collect();
        let mut pu = PerUnit::default();
        let mut arb = Arbiter4::<16, 32, 4, 2, 5, 0>::default();
        // Every host asks for one word, over and over, for as long as
        // the run lasts.
        let client = |host: Client, n: Rc<RefCell<usize>>| async move {
            loop {
                let p = host.read(Rd::at(0x10u32, 1)).await;
                let r = p.done().await;
                assert_eq!(r.resp, Resp::Okay);
                *n.borrow_mut() += 1;
            }
        };
        let mut it = links.into_iter();
        let (l0, l1, l2, l3) = (
            it.next().unwrap(),
            it.next().unwrap(),
            it.next().unwrap(),
            it.next().unwrap(),
        );
        let (mut h3, mut h2, mut h1, mut h0) = (
            hs.pop().unwrap(),
            hs.pop().unwrap(),
            hs.pop().unwrap(),
            hs.pop().unwrap(),
        );
        let hardware = join2(
            join2(
                join2(
                    h0.run(l0.host_in, l0.host_out),
                    h1.run(l1.host_in, l1.host_out),
                ),
                join2(
                    h2.run(l2.host_in, l2.host_out),
                    h3.run(l3.host_in, l3.host_out),
                ),
            ),
            join2(
                pu.run(lp.per_in, lp.per_out),
                arb.run(
                    (
                        l0.per_in.0,
                        l0.per_in.1,
                        l0.per_in.2,
                        l1.per_in.0,
                        l1.per_in.1,
                        l1.per_in.2,
                        l2.per_in.0,
                        l2.per_in.1,
                        l2.per_in.2,
                        l3.per_in.0,
                        l3.per_in.1,
                        l3.per_in.2,
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
                        l2.per_out.2,
                        l2.per_out.3,
                        l3.per_out.2,
                        l3.per_out.3,
                    ),
                ),
            ),
        );
        let clients = join2(
            join2(
                client(l0.host, counts[0].clone()),
                client(l1.host, counts[1].clone()),
            ),
            join2(
                client(l2.host, counts[2].clone()),
                client(l3.host, counts[3].clone()),
            ),
        );
        let mut sim = Running::new(join2(
            join2(hardware, memory(lp.per, Seen::default())),
            clients,
        ));
        for _ in 0..1200 {
            sim.cycle();
        }
        let got: Vec<usize> = counts.iter().map(|c| *c.borrow()).collect();
        let least = *got.iter().min().unwrap();
        let most = *got.iter().max().unwrap();
        assert!(least > 3, "every host got several bursts through: {got:?}");
        assert!(
            most - least <= 1,
            "and none was starved while another kept asking: {got:?}"
        );
    }
}
