// SPDX-License-Identifier: Apache-2.0
//! The AXI router: one host, several peripherals. It sits in the five
//! channels an [`axi`](super::axi) link makes, so nothing on either
//! side changes when it is put there. A peripheral's address range is
//! a base and a mask stated where the type is named, the decode is on
//! the address of each phase, and a burst that matches no range is
//! answered `DecErr` by the router rather than dropped.
//!
//! One count of peripherals is one unit, written out by `router!`,
//! since the lowering reads a body and not a loop over ports.
use txhdl::router;

// begin{part}
router!(Router2, 2);
router!(Router3, 3);
router!(Router4, 4);
router!(Router5, 5);
router!(Router6, 6);
router!(Router7, 7);
router!(Router8, 8);
// end{part}

/// The router against the rules it has to keep: a burst reaches the
/// range its address falls in, an address that is nobody's is
/// answered by the router, and the beats of two write bursts to two
/// peripherals do not interleave.
#[cfg(test)]
mod tests {
    use super::Router2;
    use crate::bus::axi::{
        axi, AxiHost, AxiPer, Link, Per, Rd, Resp, Wr, Xact,
    };
    use std::cell::RefCell;
    use std::rc::Rc;
    use txhdl::comp::{join2, Running, Unit};
    use txhdl::types::U;

    type Host2 = AxiHost<16, 32, 4, 3, 8>;
    type Per2 = AxiPer<16, 32, 4, 3>;
    type Job = Xact<16, 32, 4, 3>;
    /// Two peripherals, a nibble each; everything else is a hole.
    type Rtr = Router2<16, 32, 4, 3, 0x1000, 0xf000, 0x2000, 0xf000>;

    /// What a peripheral was asked to write, in the order its beats
    /// arrived: the record the interleaving test reads.
    #[derive(Clone, Default)]
    struct Seen(Rc<RefCell<Vec<(u128, Vec<u128>)>>>);

    /// A peripheral that answers a read with its own number in the
    /// high half and records every write it is given.
    fn peripheral(
        k: u128,
        per: Per<16, 32, 4, 3>,
        seen: Seen,
    ) -> impl std::future::Future<Output = ()> {
        crate::bus::axi::serve(per, 2, move |x: Job| {
            let seen = seen.clone();
            async move {
                match x {
                    Xact::Read(rd) => {
                        let at = rd.addr().raw();
                        let words: Vec<U<32>> = (0..rd.words())
                            .map(|i| {
                                U::from(
                                    ((k << 16) | ((at + i as u128 * 4) & 0xfff))
                                        as u32,
                                )
                            })
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

    /// Build a host link, two peripheral links and a router between
    /// them, run the client for `n` cycles, and hand back what each
    /// peripheral was written.
    fn drive(
        host_of: impl FnOnce(
            crate::bus::axi::Host<16, 32, 4, 3, 8>,
        )
            -> Box<dyn std::future::Future<Output = ()> + Unpin>,
        n: usize,
    ) -> (Seen, Seen) {
        let Link {
            host,
            host_in,
            host_out,
            per_in: haw,
            per_out: hbr,
            ..
        } = axi::<16, 32, 4, 3, 8>();
        let Link {
            per: p0,
            per_in: pin0,
            per_out: pout0,
            host_in: hin0,
            host_out: hout0,
            ..
        } = axi::<16, 32, 4, 3, 8>();
        let Link {
            per: p1,
            per_in: pin1,
            per_out: pout1,
            host_in: hin1,
            host_out: hout1,
            ..
        } = axi::<16, 32, 4, 3, 8>();
        let (s0, s1) = (Seen::default(), Seen::default());
        let mut hu = Host2::default();
        let mut pu0 = Per2::default();
        let mut pu1 = Per2::default();
        let mut rtr = Rtr::default();
        let hardware = join2(
            join2(
                hu.run(host_in, host_out),
                rtr.run(
                    (haw.0, haw.1, haw.2, hin0.2, hin0.3, hin1.2, hin1.3),
                    (
                        hout0.0, hout0.1, hout0.2, hout1.0, hout1.1, hout1.2,
                        hbr.2, hbr.3,
                    ),
                ),
            ),
            join2(pu0.run(pin0, pout0), pu1.run(pin1, pout1)),
        );
        let servers =
            join2(peripheral(1, p0, s0.clone()), peripheral(2, p1, s1.clone()));
        let mut sim =
            Running::new(join2(hardware, join2(host_of(host), servers)));
        for _ in 0..n {
            sim.cycle();
        }
        (s0, s1)
    }

    /// A read of each range comes back from that range, and a read of
    /// an address that is nobody's comes back `DecErr` in as many
    /// beats as it asked for, so the client's gather ends.
    #[test]
    fn each_burst_reaches_its_range() {
        let out = Rc::new(RefCell::new(Vec::new()));
        let rec = out.clone();
        drive(
            move |host| {
                Box::new(Box::pin(async move {
                    let a = host.read(Rd::at(0x1004u32, 1)).await;
                    let b = host.read(Rd::at(0x2008u32, 1)).await;
                    let c = host.read(Rd::at(0x7000u32, 3)).await;
                    for p in [a, b, c] {
                        let r = p.done().await;
                        rec.borrow_mut().push((r.resp, r.data.len(), {
                            r.data.first().map(|w| w.raw()).unwrap_or(0)
                        }));
                    }
                }))
            },
            120,
        );
        let got = out.borrow().clone();
        assert_eq!(got.len(), 3, "every burst answered");
        assert_eq!(got[0], (Resp::Okay, 1, 0x10004), "the first range");
        assert_eq!(got[1], (Resp::Okay, 1, 0x20008), "the second range");
        assert_eq!(
            (got[2].0, got[2].1),
            (Resp::DecErr, 3),
            "a hole answers itself, in the beats it was asked for"
        );
    }

    /// Two write bursts of several beats, to two peripherals, issued
    /// one after the other. AXI4 puts no identifier on the write data
    /// channel, so the router must keep each burst's beats with the
    /// address phase they followed; if it did not, each peripheral
    /// would be given a mixture of the two.
    #[test]
    fn write_beats_do_not_interleave() {
        let (s0, s1) = drive(
            |host| {
                Box::new(Box::pin(async move {
                    let a = host
                        .write(
                            Wr::at(0x1000u32),
                            &[
                                U::from(0xa0u32),
                                U::from(0xa1u32),
                                U::from(0xa2u32),
                            ],
                        )
                        .await;
                    let b = host
                        .write(
                            Wr::at(0x2000u32),
                            &[
                                U::from(0xb0u32),
                                U::from(0xb1u32),
                                U::from(0xb2u32),
                            ],
                        )
                        .await;
                    let ra = a.done().await;
                    let rb = b.done().await;
                    assert_eq!(ra.resp, Resp::Okay);
                    assert_eq!(rb.resp, Resp::Okay);
                }))
            },
            160,
        );
        let first = s0.0.borrow().clone();
        let second = s1.0.borrow().clone();
        assert_eq!(first.len(), 1, "the first peripheral saw one burst");
        assert_eq!(second.len(), 1, "the second peripheral saw one burst");
        assert_eq!(
            first[0],
            (0x1000, vec![0xa0, 0xa1, 0xa2]),
            "the first burst's beats, whole and in order"
        );
        assert_eq!(
            second[0],
            (0x2000, vec![0xb0, 0xb1, 0xb2]),
            "the second burst's beats, whole and in order"
        );
    }
}
