// SPDX-License-Identifier: Apache-2.0
//! The AXI router: one host, several peripherals. It sits in the five
//! channels an [`axi`](super::axi) link makes, so nothing on either
//! side changes when it is put there. A peripheral's address range is
//! a base and a mask, stated as a map type where the router is named,
//! the decode is on the address of each phase, and a burst that
//! matches no range is answered `DecErr` by the router rather than
//! dropped.
//!
//! One unit serves every count of peripherals: its peripheral sides
//! are arrays of ports, the lowering unrolls the loops over them when
//! `lowered` runs (issue 500), and the ranges are the map's constant
//! array, read the same way (issue 593). Until then a router per count
//! was text `router!` wrote, `Router2` to `Router8`.
use crate::bus::axi::{Ar, Aw, Resp, B, R, W};
use std::marker::PhantomData;
use txhdl::comp::{mux, Clock, DefaultClock, Reg, Rx, Tx, Unit};
use txhdl::map::AddrMap;
use txhdl::types::{Bit, U};
use txhdl::{lower, with, Trace};

// begin{part}
/// An AXI4 router of one host and `N` peripherals. The host's five
/// channels come in; five go out per peripheral, and a peripheral's
/// range is its entry of `M::RANGES`, a base and a mask matched on the
/// address of each phase, the first that matches the one. A burst
/// whose address is no peripheral's is answered `DecErr` by the router
/// itself rather than dropped. Write beats follow the address phase
/// they belong to, and a second write waits for the first burst's
/// beats, since AXI4 puts no identifier on `w`; a read burst's beats
/// stay together, so `last` still means what it says. Responses merge
/// back to the host, the lowest peripheral first when two answer at
/// once. Written in the lowered subset, so it is a netlist too.
// begin{state}
#[derive(Trace)]
pub struct Router<
    const N: usize,
    M: AddrMap<N>,
    const A: usize,
    const D: usize,
    const S: usize,
    const I: usize,
> {
    /// The map, which holds no signal: a marker the netlist ignores.
    pub map: PhantomData<M>,
    /// A write burst's beats are going out; a second address phase
    /// waits, since AXI4 puts no identifier on `w`.
    pub wbusy: Reg<Bit>,
    /// Where this write burst's beats go, one bit per peripheral;
    /// none set means they are being swallowed.
    pub wsel: Reg<U<N>>,
    /// This write burst decoded to no peripheral.
    pub whole: Reg<Bit>,
    /// Its identifier, to answer when its last beat has gone.
    pub wid: Reg<U<I>>,
    /// A write response the router owes for a decode error.
    pub berr: Reg<Bit>,
    /// The identifier that response must carry.
    pub berr_id: Reg<U<I>>,
    /// A read burst is going up; its beats stay together.
    pub rbusy: Reg<Bit>,
    /// Which peripheral that burst is coming from, one bit each, so
    /// no other one's beats get in between.
    pub rsel: Reg<U<N>>,
    /// A read burst the router owes for a decode error.
    pub rerr: Reg<Bit>,
    /// The identifier its beats must carry.
    pub rerr_id: Reg<U<I>>,
    /// How many of its beats are still to send. A read of a hole is
    /// answered in as many beats as it asked for, or the client would
    /// gather for ever.
    pub rerr_left: Reg<U<9>>,
}
// end{state}

impl<
        const N: usize,
        M: AddrMap<N>,
        const A: usize,
        const D: usize,
        const S: usize,
        const I: usize,
    > Default for Router<N, M, A, D, S, I>
{
    fn default() -> Self {
        Router {
            map: PhantomData,
            wbusy: Reg::default(),
            wsel: Reg::default(),
            whole: Reg::default(),
            wid: Reg::default(),
            berr: Reg::default(),
            berr_id: Reg::default(),
            rbusy: Reg::default(),
            rsel: Reg::default(),
            rerr: Reg::default(),
            rerr_id: Reg::default(),
            rerr_left: Reg::default(),
        }
    }
}

// The lowering reads a loop over an array of ports as `aws[i]`, so the
// index is what it is written with, and Clippy would rather it were an
// iterator.
#[allow(clippy::needless_range_loop)]
#[lower]
impl<
        const N: usize,
        M: AddrMap<N>,
        const A: usize,
        const D: usize,
        const S: usize,
        const I: usize,
    > Unit for Router<N, M, A, D, S, I>
{
    async fn run(
        &mut self,
        (aw, ar, w, bs, rs): (
            Rx<Aw<A, I>>,
            Rx<Ar<A, I>>,
            Rx<W<D, S>>,
            [Rx<B<I>>; N],
            [Rx<R<D, I>>; N],
        ),
        (aws, ars, ws, b, r): (
            [Tx<Aw<A, I>>; N],
            [Tx<Ar<A, I>>; N],
            [Tx<W<D, S>>; N],
            Tx<B<I>>,
            Tx<R<D, I>>,
        ),
    ) {
        loop {
            DefaultClock::rising().await;
            let wbusy = self.wbusy.get();
            let wsel = self.wsel.get();
            let rbusy = self.rbusy.get();
            let rsel = self.rsel.get();
            let berr = self.berr.get();
            let rerr = self.rerr.get();
            // begin{decode}
            // Where the write address at the head would go, one bit
            // per peripheral for the first range that holds it, and
            // whether that peripheral has room for it; a hole has
            // room when no error response is owed.
            let aw_off = Bit::from(aw.peek().is_some());
            let awh = aw.head();
            let mut aw_any = Bit::Zero;
            let mut aw_first = U::<N>::from(0u8);
            let mut aw_room = !berr;
            for i in 0..N {
                let hit = Bit::from(
                    (awh.addr.raw() as usize & M::RANGES[i].1)
                        == M::RANGES[i].0,
                ) & !aw_any;
                let one = U::<N>::from(1u8) << i;
                aw_first = mux(hit, one, aw_first);
                aw_room = mux(hit, aws[i].ready(), aw_room);
                aw_any = aw_any | hit;
            }
            let aw_hole = !aw_any;
            let take_aw = aw_off & !wbusy & aw_room;
            let _ = aw.recv_if(!wbusy & aw_room);
            for i in 0..N {
                if (take_aw & aw_first.bit(i)).to_bool() {
                    aws[i].send(awh);
                }
            }
            // The read address phase, the same way. Several reads may
            // be outstanding, since each carries its identifier.
            let ar_off = Bit::from(ar.peek().is_some());
            let arh = ar.head();
            let mut ar_any = Bit::Zero;
            let mut ar_first = U::<N>::from(0u8);
            let mut ar_room = !rerr;
            for i in 0..N {
                let hit = Bit::from(
                    (arh.addr.raw() as usize & M::RANGES[i].1)
                        == M::RANGES[i].0,
                ) & !ar_any;
                let one = U::<N>::from(1u8) << i;
                ar_first = mux(hit, one, ar_first);
                ar_room = mux(hit, ars[i].ready(), ar_room);
                ar_any = ar_any | hit;
            }
            let ar_hole = !ar_any;
            let take_ar = ar_off & ar_room;
            let _ = ar.recv_if(ar_room);
            for i in 0..N {
                if (take_ar & ar_first.bit(i)).to_bool() {
                    ars[i].send(arh);
                }
            }
            // end{decode}
            // begin{beats}
            // The write beats go where this burst's address went; a
            // burst to a hole is swallowed here and answered when its
            // last beat has gone.
            let w_off = Bit::from(w.peek().is_some());
            let wh = w.head();
            let mut w_room = Bit::One;
            for i in 0..N {
                w_room = mux(wsel.bit(i), ws[i].ready(), w_room);
            }
            let w_go = wbusy & w_off & w_room;
            let _ = w.recv_if(wbusy & w_room);
            for i in 0..N {
                if (w_go & wsel.bit(i)).to_bool() {
                    ws[i].send(wh);
                }
            }
            let w_done = w_go & wh.last;
            // end{beats}
            // begin{merge}
            // The read beats going up: the burst already going stays
            // chosen, so its beats stay together; a decode error's own
            // burst goes before a new one, and among the peripherals
            // offering, the lowest goes first.
            let r_room = r.ready();
            let err_turn = rerr & !rbusy;
            let mut r_seen = Bit::Zero;
            let mut r_from_per = Bit::Zero;
            let mut rsel_new = U::<N>::from(0u8);
            let mut sel_id = rs[0].head().id;
            let mut sel_data = rs[0].head().data;
            let mut sel_resp = rs[0].head().resp;
            let mut sel_last = rs[0].head().last;
            for i in 0..N {
                let off = Bit::from(rs[i].peek().is_some());
                let pick = mux(rbusy, rsel.bit(i), !err_turn & !r_seen & off);
                r_seen = r_seen | off;
                let _ = rs[i].recv_if(pick & r_room);
                r_from_per = r_from_per | (pick & off);
                let one = U::<N>::from(1u8) << i;
                rsel_new = mux(pick, one, rsel_new);
                sel_id = mux(pick, rs[i].head().id, sel_id);
                sel_data = mux(pick, rs[i].head().data, sel_data);
                sel_resp = mux(pick, rs[i].head().resp, sel_resp);
                sel_last = mux(pick, rs[i].head().last, sel_last);
            }
            let r_go = r_from_per & r_room;
            let err_last = self.rerr_left.get() == 1;
            let err_go = err_turn & r_room;
            // The beat that goes up, from the chosen peripheral or
            // from the router's own error.
            let up_id = mux(err_turn, self.rerr_id.get(), sel_id);
            let up_data = mux(err_turn, U::<D>::from(0u8), sel_data);
            let up_resp = mux(err_turn, Resp::DecErr, sel_resp);
            let per_last = sel_last;
            let up_last = mux(err_turn, Bit::from(err_last), per_last);
            if (err_go | r_go).to_bool() {
                r.send(R {
                    id: up_id,
                    data: up_data,
                    resp: up_resp,
                    last: up_last,
                });
            }
            // The write responses, the same choice, with the router's
            // own decode error first.
            let b_room = b.ready();
            let mut b_seen = Bit::Zero;
            let mut b_any = Bit::Zero;
            let mut selb_id = bs[0].head().id;
            let mut selb_resp = bs[0].head().resp;
            for i in 0..N {
                let off = Bit::from(bs[i].peek().is_some());
                let pick = !berr & !b_seen & off;
                b_seen = b_seen | off;
                let _ = bs[i].recv_if(pick & b_room);
                b_any = b_any | pick;
                selb_id = mux(pick, bs[i].head().id, selb_id);
                selb_resp = mux(pick, bs[i].head().resp, selb_resp);
            }
            let b_go = b_any & b_room;
            let berr_go = berr & b_room;
            let upb_id = mux(berr, self.berr_id.get(), selb_id);
            let upb_resp = mux(berr, Resp::DecErr, selb_resp);
            if (b_go | berr_go).to_bool() {
                b.send(B {
                    id: upb_id,
                    resp: upb_resp,
                });
            }
            // end{merge}
            // begin{drives}
            with!(self <= {
                take_aw ? {
                    wbusy: Bit::One,
                    wsel: aw_first,
                    whole: aw_hole,
                    wid: awh.id,
                },
                w_done ? { wbusy: Bit::Zero },
                (w_done & self.whole.get()) ? {
                    berr: Bit::One,
                    berr_id: self.wid.get(),
                },
                berr_go ? { berr: Bit::Zero },
                (take_ar & ar_hole) ? {
                    rerr: Bit::One,
                    rerr_id: arh.id,
                    rerr_left: arh.len.zext::<9>() + 1,
                },
                err_go ? {
                    rerr_left: self.rerr_left.get() - 1,
                    rerr: !err_last,
                },
                r_go ? { rbusy: !per_last, rsel: rsel_new },
            });
            // end{drives}
        }
    }
}
// end{part}

/// The router against the rules it has to keep: a burst reaches the
/// range its address falls in, an address that is nobody's is
/// answered by the router, and the beats of two write bursts to two
/// peripherals do not interleave.
#[cfg(test)]
mod tests {
    use super::Router;
    use crate::bus::axi::{
        axi, AxiHost, AxiPer, Link, Per, Rd, Resp, Wr, Xact,
    };
    use std::cell::RefCell;
    use std::rc::Rc;
    use txhdl::comp::{join2, Running, Unit};
    use txhdl::map::AddrMap;
    use txhdl::types::U;

    type Host2 = AxiHost<16, 32, 4, 3, 8>;
    type Per2 = AxiPer<16, 32, 4, 3>;
    type Job = Xact<16, 32, 4, 3>;
    /// Two peripherals, a nibble each; everything else is a hole.
    struct TwoMap;

    impl AddrMap<2> for TwoMap {
        const RANGES: [(usize, usize); 2] =
            [(0x1000, 0xf000), (0x2000, 0xf000)];
    }

    type Rtr = Router<2, TwoMap, 16, 32, 4, 3>;

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
                    (haw.0, haw.1, haw.2, [hin0.2, hin1.2], [hin0.3, hin1.3]),
                    (
                        [hout0.0, hout1.0],
                        [hout0.1, hout1.1],
                        [hout0.2, hout1.2],
                        hbr.2,
                        hbr.3,
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
