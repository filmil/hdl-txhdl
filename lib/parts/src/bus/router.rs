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
use crate::bus::{IsMap, Map};
use std::marker::PhantomData;
use txhdl::comp::{mux, Clock, DefaultClock, Reg, Rx, Tx, Unit};
use txhdl::map::AddrMap;
use txhdl::types::{Bit, U};
use txhdl::{lower, named_type_params, with, Trace};

// begin{part}
/// An AXI4 router of one host and `PORTS` peripherals. The host's five
/// channels come in; five go out per peripheral, and a peripheral's
/// range is its entry of `Map::RANGES`, a base and a mask matched on the
/// address of each phase, the first that matches the one. A burst
/// whose address is no peripheral's is answered `DecErr` by the router
/// itself rather than dropped. Write beats follow the address phase
/// they belong to, and a second write waits for the first burst's
/// beats, since AXI4 puts no identifier on `w`; a read burst's beats
/// stay together, so `last` still means what it says. Responses merge
/// back to the host, the lowest peripheral first when two answer at
/// once. Written in the lowered subset, so it is a netlist too.
// begin{state}
#[named_type_params(reuse(crate::bus::named::Map), path = crate::bus::router)]
#[derive(Trace)]
pub struct Router<
    const PORTS: usize,
    Map,
    const ADDR: usize,
    const DATA: usize,
    const STRB: usize,
    const ID: usize,
> where
    Map: AddrMap<PORTS>,
{
    /// The map, which holds no signal: a marker the netlist ignores.
    pub map: PhantomData<Map>,
    /// A write burst's beats are going out; a second address phase
    /// waits, since AXI4 puts no identifier on `w`.
    pub wbusy: Reg<Bit>,
    /// Where this write burst's beats go, one bit per peripheral;
    /// none set means they are being swallowed.
    pub wsel: Reg<U<PORTS>>,
    /// This write burst decoded to no peripheral.
    pub whole: Reg<Bit>,
    /// Its identifier, to answer when its last beat has gone.
    pub wid: Reg<U<ID>>,
    /// A write response the router owes for a decode error.
    pub berr: Reg<Bit>,
    /// The identifier that response must carry.
    pub berr_id: Reg<U<ID>>,
    /// A read burst is going up; its beats stay together.
    pub rbusy: Reg<Bit>,
    /// Which peripheral that burst is coming from, one bit each, so
    /// no other one's beats get in between.
    pub rsel: Reg<U<PORTS>>,
    /// A read burst the router owes for a decode error.
    pub rerr: Reg<Bit>,
    /// The identifier its beats must carry.
    pub rerr_id: Reg<U<ID>>,
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
    > Default for Router<N, Map<M>, A, D, S, I>
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
    > Unit for Router<N, Map<M>, A, D, S, I>
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

// begin{halves}
/// The request half of [`Router`]: the decode of each address phase,
/// the write beats that follow theirs, and the answers the router
/// itself owes for an address that is nobody's. Those go out on a link
/// of their own, `eb` and `er`, which [`RouterRsp`] merges with the
/// peripherals' answers. The parameters are [`Router`]'s.
///
/// With [`RouterRsp`] it is the router in two units, for a design
/// whose answers cross the router over unregistered channels as its
/// requests do (issue 2077): an unregistered channel's sender runs
/// first, and one unit cannot run both before the peripherals and
/// after them. Such a design joins this half before the peripherals
/// and the answer half after them, and makes `eb` and `er` between the
/// two. [`Router`] itself is one unit, as it was.
///
/// Joined so, with `eb` and `er` unregistered, the two answer a burst
/// to a peripheral in the cycles [`Router`] does, and a burst to a hole
/// too while nothing holds its answer up. They differ where the router
/// waits: it takes no second address phase to a hole while it still
/// owes the first one's answer. This half owes an answer only until it
/// is in `eb` or `er`, which hold two beats each. So while the answer
/// half is in a peripheral's read burst, or the host is not taking
/// answers, the next address phase to a hole can be taken sooner than
/// the router would take it, and its answer can follow the first one's
/// without the gap the router leaves. The answers still go up whole
/// and in order.
#[named_type_params(reuse(crate::bus::named::Map), path = crate::bus::router)]
#[derive(Trace)]
pub struct RouterReq<
    const PORTS: usize,
    Map,
    const ADDR: usize,
    const DATA: usize,
    const STRB: usize,
    const ID: usize,
> where
    Map: AddrMap<PORTS>,
{
    /// The map, which holds no signal: a marker the netlist ignores.
    pub map: PhantomData<Map>,
    /// A write burst's beats are going out; a second address phase
    /// waits, since AXI4 puts no identifier on `w`.
    pub wbusy: Reg<Bit>,
    /// Where this write burst's beats go, one bit per peripheral;
    /// none set means they are being swallowed.
    pub wsel: Reg<U<PORTS>>,
    /// This write burst decoded to no peripheral.
    pub whole: Reg<Bit>,
    /// Its identifier, to answer when its last beat has gone.
    pub wid: Reg<U<ID>>,
    /// A write response this half owes for a decode error.
    pub berr: Reg<Bit>,
    /// The identifier that response must carry.
    pub berr_id: Reg<U<ID>>,
    /// A read burst this half owes for a decode error.
    pub rerr: Reg<Bit>,
    /// The identifier its beats must carry.
    pub rerr_id: Reg<U<ID>>,
    /// How many of its beats are still to send. A read of a hole is
    /// answered in as many beats as it asked for, or the client would
    /// gather for ever.
    pub rerr_left: Reg<U<9>>,
}

impl<
        const N: usize,
        M: AddrMap<N>,
        const A: usize,
        const D: usize,
        const S: usize,
        const I: usize,
    > Default for RouterReq<N, Map<M>, A, D, S, I>
{
    fn default() -> Self {
        RouterReq {
            map: PhantomData,
            wbusy: Reg::default(),
            wsel: Reg::default(),
            whole: Reg::default(),
            wid: Reg::default(),
            berr: Reg::default(),
            berr_id: Reg::default(),
            rerr: Reg::default(),
            rerr_id: Reg::default(),
            rerr_left: Reg::default(),
        }
    }
}

#[allow(clippy::needless_range_loop)]
#[lower]
impl<
        const N: usize,
        M: AddrMap<N>,
        const A: usize,
        const D: usize,
        const S: usize,
        const I: usize,
    > Unit for RouterReq<N, Map<M>, A, D, S, I>
{
    async fn run(
        &mut self,
        (aw, ar, w): (Rx<Aw<A, I>>, Rx<Ar<A, I>>, Rx<W<D, S>>),
        (aws, ars, ws, eb, er): (
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
            let berr = self.berr.get();
            let rerr = self.rerr.get();
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
            // The answers this half owes, onto their own link: a
            // write's when the burst's last beat has gone, a read's a
            // beat a cycle, as many as the burst asked for.
            let berr_go = berr & eb.ready();
            if berr_go.to_bool() {
                eb.send(B {
                    id: self.berr_id.get(),
                    resp: Resp::DecErr,
                });
            }
            let err_last = self.rerr_left.get() == 1;
            let err_go = rerr & er.ready();
            if err_go.to_bool() {
                er.send(R {
                    id: self.rerr_id.get(),
                    data: U::<D>::from(0u8),
                    resp: Resp::DecErr,
                    last: Bit::from(err_last),
                });
            }
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
            });
        }
    }
}

/// The answer half of [`Router`]: the peripherals' `B` beats and `R`
/// beats merged back to the host, with the request half's own answers,
/// which come in on `eb` and `er` and go first. A read burst's beats
/// stay together, whichever of them it comes from; among the
/// peripherals offering, the lowest goes first. `PORTS`, `DATA` and
/// `ID` are [`Router`]'s.
#[named_type_params(path = crate::bus::router)]
#[derive(Trace, Default)]
pub struct RouterRsp<const PORTS: usize, const DATA: usize, const ID: usize> {
    /// A read burst is going up; its beats stay together.
    pub rbusy: Reg<Bit>,
    /// Which peripheral that burst is coming from, one bit each, so
    /// no other one's beats get in between; none set while it is the
    /// request half's.
    pub rsel: Reg<U<PORTS>>,
    /// That burst is the request half's own.
    pub resel: Reg<Bit>,
}

#[allow(clippy::needless_range_loop)]
#[lower]
impl<const N: usize, const D: usize, const I: usize> Unit
    for RouterRsp<N, D, I>
{
    async fn run(
        &mut self,
        (bs, rs, eb, er): (
            [Rx<B<I>>; N],
            [Rx<R<D, I>>; N],
            Rx<B<I>>,
            Rx<R<D, I>>,
        ),
        (b, r): (Tx<B<I>>, Tx<R<D, I>>),
    ) {
        loop {
            DefaultClock::rising().await;
            let rbusy = self.rbusy.get();
            let rsel = self.rsel.get();
            // The read beats going up: the burst already going stays
            // chosen, so its beats stay together; the request half's
            // own burst goes before a new one, and among the
            // peripherals offering, the lowest goes first.
            let r_room = r.ready();
            let e_off = Bit::from(er.peek().is_some());
            let e_pick = mux(rbusy, self.resel.get(), e_off);
            let _ = er.recv_if(e_pick & r_room);
            let mut r_seen = e_off;
            let mut r_from = e_pick & e_off;
            let mut rsel_new = U::<N>::from(0u8);
            let mut sel_id = er.head().id;
            let mut sel_data = er.head().data;
            let mut sel_resp = er.head().resp;
            let mut sel_last = er.head().last;
            for i in 0..N {
                let off = Bit::from(rs[i].peek().is_some());
                let pick = mux(rbusy, rsel.bit(i), !r_seen & off);
                r_seen = r_seen | off;
                let _ = rs[i].recv_if(pick & r_room);
                r_from = r_from | (pick & off);
                let one = U::<N>::from(1u8) << i;
                rsel_new = mux(pick, one, rsel_new);
                sel_id = mux(pick, rs[i].head().id, sel_id);
                sel_data = mux(pick, rs[i].head().data, sel_data);
                sel_resp = mux(pick, rs[i].head().resp, sel_resp);
                sel_last = mux(pick, rs[i].head().last, sel_last);
            }
            let r_go = r_from & r_room;
            if r_go.to_bool() {
                r.send(R {
                    id: sel_id,
                    data: sel_data,
                    resp: sel_resp,
                    last: sel_last,
                });
            }
            // The write responses, the same choice, the request half's
            // own first.
            let b_room = b.ready();
            let eb_off = Bit::from(eb.peek().is_some());
            let _ = eb.recv_if(b_room);
            let mut b_seen = eb_off;
            let mut b_any = eb_off;
            let mut selb_id = eb.head().id;
            let mut selb_resp = eb.head().resp;
            for i in 0..N {
                let off = Bit::from(bs[i].peek().is_some());
                let pick = !b_seen & off;
                b_seen = b_seen | off;
                let _ = bs[i].recv_if(pick & b_room);
                b_any = b_any | pick;
                selb_id = mux(pick, bs[i].head().id, selb_id);
                selb_resp = mux(pick, bs[i].head().resp, selb_resp);
            }
            let b_go = b_any & b_room;
            if b_go.to_bool() {
                b.send(B {
                    id: selb_id,
                    resp: selb_resp,
                });
            }
            with!(self <= {
                r_go ? {
                    rbusy: !sel_last,
                    rsel: rsel_new,
                    resel: e_pick,
                },
            });
        }
    }
}
// end{halves}

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

    type Host2 = AxiHost![ADDR = 16, DATA = 32, STRB = 4, ID = 3, IDS = 8];
    type Per2 = AxiPer![ADDR = 16, DATA = 32, STRB = 4, ID = 3];
    type Job = Xact<16, 32, 4, 3>;
    /// Two peripherals, a nibble each; everything else is a hole.
    struct TwoMap;

    impl AddrMap<2> for TwoMap {
        const RANGES: [(usize, usize); 2] =
            [(0x1000, 0xf000), (0x2000, 0xf000)];
    }

    type Rtr = Router![
        PORTS = 2,
        Map = TwoMap,
        ADDR = 16,
        DATA = 32,
        STRB = 4,
        ID = 3
    ];

    /// What a peripheral was asked to write, in the order its beats
    /// arrived: the record the interleaving test reads.
    #[derive(Clone, Default)]
    struct Seen(Rc<RefCell<Vec<(u128, Vec<u128>)>>>);

    /// A peripheral that answers a read with its own number in the
    /// high half and records every write it is given.
    fn peripheral(
        k: u128,
        per: Per![ADDR = 16, DATA = 32, STRB = 4, ID = 3],
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
    // The halves (issue 2077).
    use super::{RouterReq, RouterRsp};
    use crate::bus::arbiter::{Arbiter2, ArbiterReq, ArbiterRsp};
    use crate::bus::axi::{Ar, Aw, AxiPerReq, AxiPerRsp, B, R, W};
    use txhdl::comp::{
        chan, chan_unregistered, now, Clock, DefaultClock, Rx, Tx,
    };
    use txhdl::types::Bit;

    type Req2 = RouterReq![
        PORTS = 2,
        Map = TwoMap,
        ADDR = 16,
        DATA = 32,
        STRB = 4,
        ID = 3
    ];
    type Rsp2 = RouterRsp![PORTS = 2, DATA = 32, ID = 3];
    type PerReq2 = AxiPerReq![ADDR = 16, DATA = 32, STRB = 4, ID = 3];
    type PerRsp2 = AxiPerRsp![ADDR = 16, DATA = 32, STRB = 4, ID = 3];
    type HostA = AxiHost![ADDR = 16, DATA = 32, STRB = 4, ID = 2, IDS = 4];
    type Arb2 = Arbiter2![
        ADDR = 16,
        DATA = 32,
        STRB = 4,
        ID = 2,
        PER_ID = 3,
        FIXED = 0
    ];
    type ArbReq2 = ArbiterReq![
        HOSTS = 2,
        ADDR = 16,
        DATA = 32,
        STRB = 4,
        ID = 2,
        PER_ID = 3,
        FIXED = 0
    ];
    type ArbRsp2 = ArbiterRsp![HOSTS = 2, DATA = 32, ID = 2, PER_ID = 3];

    /// A channel, unregistered when `$u` says so.
    macro_rules! ch {
        ($u:expr, $t:ty) => {
            if $u {
                chan_unregistered::<$t, DefaultClock>()
            } else {
                chan::<$t, DefaultClock>()
            }
        };
    }

    /// `drive`, with the router as its two halves joined, the request
    /// half first and its own answers' link unregistered.
    fn drive_halves(
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
        let (eb_tx, eb_rx) = chan_unregistered::<B<3>, DefaultClock>();
        let (er_tx, er_rx) = chan_unregistered::<R<32, 3>, DefaultClock>();
        let (s0, s1) = (Seen::default(), Seen::default());
        let mut hu = Host2::default();
        let mut pu0 = Per2::default();
        let mut pu1 = Per2::default();
        let mut req = Req2::default();
        let mut rsp = Rsp2::default();
        let hardware = join2(
            join2(
                hu.run(host_in, host_out),
                join2(
                    req.run(
                        (haw.0, haw.1, haw.2),
                        (
                            [hout0.0, hout1.0],
                            [hout0.1, hout1.1],
                            [hout0.2, hout1.2],
                            eb_tx,
                            er_tx,
                        ),
                    ),
                    rsp.run(
                        ([hin0.2, hin1.2], [hin0.3, hin1.3], eb_rx, er_rx),
                        (hbr.2, hbr.3),
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

    /// What a client saw of four bursts, each with the cycle its answer
    /// was whole: a read of each range, a read of a hole, and a write
    /// to a hole.
    type Log = Rc<RefCell<Vec<(Resp, usize, u128, u64)>>>;

    fn four_bursts(
        rec: Log,
    ) -> impl FnOnce(
        crate::bus::axi::Host<16, 32, 4, 3, 8>,
    ) -> Box<dyn std::future::Future<Output = ()> + Unpin> {
        move |host| {
            Box::new(Box::pin(async move {
                // Time runs on from one run to the next in a test, so
                // each answer's cycle is counted from this run's start.
                let t0 = now();
                let a = host.read(Rd::at(0x1004u32, 1)).await;
                let b = host.read(Rd::at(0x2008u32, 2)).await;
                let c = host.read(Rd::at(0x7000u32, 3)).await;
                // Each read is waited for by itself, so its cycle is
                // the cycle its own last beat came, whatever the order.
                let wait = |k: usize, p: crate::bus::axi::Pending<32, 3>| {
                    let rec = rec.clone();
                    async move {
                        let r = p.done().await;
                        let first =
                            r.data.first().map(|w| w.raw()).unwrap_or(0);
                        let at = (now() - t0) / DefaultClock::PERIOD;
                        rec.borrow_mut().push((
                            r.resp,
                            r.data.len(),
                            first + ((k as u128) << 64),
                            at,
                        ));
                    }
                };
                join2(wait(0, a), join2(wait(1, b), wait(2, c))).await;
                let w = host
                    .write(Wr::at(0x7000u32), &[U::from(1u32), U::from(2u32)])
                    .await;
                let r = w.done().await;
                let at = (now() - t0) / DefaultClock::PERIOD;
                rec.borrow_mut().push((r.resp, 0, 3 << 64, at));
            }))
        }
    }

    /// A log with its bursts in the order they were issued.
    fn in_order(log: &Log) -> Vec<(Resp, usize, u128, u64)> {
        let mut v = log.borrow().clone();
        v.sort_by_key(|e| e.2 >> 64);
        v
    }

    /// The two halves joined keep the router's rules, and answer in
    /// the cycles the router does: the ranges, a read of a hole in the
    /// beats it asked for, and a write to a hole.
    #[test]
    fn the_halves_joined_answer_as_the_router_does() {
        let whole: Log = Rc::new(RefCell::new(Vec::new()));
        let halves: Log = Rc::new(RefCell::new(Vec::new()));
        drive(four_bursts(whole.clone()), 200);
        drive_halves(four_bursts(halves.clone()), 200);
        let (w, h) = (in_order(&whole), in_order(&halves));
        assert_eq!(w.len(), 4, "the router answered every burst");
        let low = |e: &(Resp, usize, u128, u64)| e.2 & 0xffff_ffff;
        assert_eq!((w[0].0, low(&w[0])), (Resp::Okay, 0x10004));
        assert_eq!((w[1].0, w[1].1, low(&w[1])), (Resp::Okay, 2, 0x20008));
        assert_eq!((w[2].0, w[2].1), (Resp::DecErr, 3));
        assert_eq!(w[3].0, Resp::DecErr, "a write to a hole");
        assert!(w[2].3 < w[1].3, "the hole's answer does not wait its turn");
        assert_eq!(h, w, "the halves: the same answers in the same cycles");
    }

    /// The halves keep each write burst's beats with its address phase.
    #[test]
    fn the_halves_do_not_interleave_write_beats() {
        let (s0, s1) = drive_halves(
            |host| {
                Box::new(Box::pin(async move {
                    let a = host
                        .write(
                            Wr::at(0x1000u32),
                            &[U::from(0xa0u32), U::from(0xa1u32)],
                        )
                        .await;
                    let b = host
                        .write(
                            Wr::at(0x2000u32),
                            &[U::from(0xb0u32), U::from(0xb1u32)],
                        )
                        .await;
                    assert_eq!(a.done().await.resp, Resp::Okay);
                    assert_eq!(b.done().await.resp, Resp::Okay);
                }))
            },
            160,
        );
        assert_eq!(s0.0.borrow().clone(), vec![(0x1000, vec![0xa0, 0xa1])]);
        assert_eq!(s1.0.borrow().clone(), vec![(0x2000, vec![0xb0, 0xb1])]);
    }

    /// The answer half alone, its ports driven by hand. A read burst
    /// that is going up stays whole when the request half's own answer
    /// and a lower peripheral start to offer in its middle; of those
    /// two, the request half's goes first.
    #[test]
    fn the_answer_half_keeps_a_read_burst_whole() {
        let (_b0_tx, b0_rx) = chan::<B<3>, DefaultClock>();
        let (_b1_tx, b1_rx) = chan::<B<3>, DefaultClock>();
        let (r0_tx, r0_rx) = chan::<R<32, 3>, DefaultClock>();
        let (r1_tx, r1_rx) = chan::<R<32, 3>, DefaultClock>();
        let (_eb_tx, eb_rx) = chan::<B<3>, DefaultClock>();
        let (er_tx, er_rx) = chan::<R<32, 3>, DefaultClock>();
        let (b_tx, _b_rx) = chan::<B<3>, DefaultClock>();
        let (r_tx, r_rx) = chan::<R<32, 3>, DefaultClock>();
        let mut rsp = Rsp2::default();
        let mut sim =
            Running::new(rsp.run(
                ([b0_rx, b1_rx], [r0_rx, r1_rx], eb_rx, er_rx),
                (b_tx, r_tx),
            ));
        // A burst of three beats under identifier `id`.
        let beat = |id: u8, k: usize| R {
            id: U::<3>::from(id),
            data: U::<32>::from(k as u32),
            resp: Resp::Okay,
            last: Bit::from(k == 2),
        };
        let (mut k0, mut k1, mut ke) = (0usize, 0usize, 0usize);
        let mut got = Vec::new();
        for t in 0..24 {
            // The second peripheral starts alone; the first one and the
            // request half's own answer come two cycles later.
            if k1 < 3 && r1_tx.ready().to_bool() {
                r1_tx.send(beat(2, k1));
                k1 += 1;
            }
            if t >= 2 && k0 < 3 && r0_tx.ready().to_bool() {
                r0_tx.send(beat(1, k0));
                k0 += 1;
            }
            if t >= 2 && ke < 3 && er_tx.ready().to_bool() {
                er_tx.send(beat(3, ke));
                ke += 1;
            }
            if let Some(x) = r_rx.recv() {
                got.push(x.id.raw() as u8);
            }
            sim.cycle();
        }
        assert_eq!(got, vec![2, 2, 2, 3, 3, 3, 1, 1, 1]);
    }

    /// The answer half again: the request half's own burst stays whole
    /// too when its beats come with a gap and a peripheral offers in the
    /// gap.
    #[test]
    fn the_answer_half_keeps_the_request_halfs_burst_whole() {
        let (_b0_tx, b0_rx) = chan::<B<3>, DefaultClock>();
        let (_b1_tx, b1_rx) = chan::<B<3>, DefaultClock>();
        let (r0_tx, r0_rx) = chan::<R<32, 3>, DefaultClock>();
        let (_r1_tx, r1_rx) = chan::<R<32, 3>, DefaultClock>();
        let (_eb_tx, eb_rx) = chan::<B<3>, DefaultClock>();
        let (er_tx, er_rx) = chan::<R<32, 3>, DefaultClock>();
        let (b_tx, _b_rx) = chan::<B<3>, DefaultClock>();
        let (r_tx, r_rx) = chan::<R<32, 3>, DefaultClock>();
        let mut rsp = Rsp2::default();
        let mut sim =
            Running::new(rsp.run(
                ([b0_rx, b1_rx], [r0_rx, r1_rx], eb_rx, er_rx),
                (b_tx, r_tx),
            ));
        let beat = |id: u8, k: usize| R {
            id: U::<3>::from(id),
            data: U::<32>::from(k as u32),
            resp: Resp::Okay,
            last: Bit::from(k == 2),
        };
        let (mut k0, mut ke) = (0usize, 0usize);
        let mut got = Vec::new();
        for t in 0..24 {
            // The request half's first beat, then nothing from it for
            // four cycles, in which the first peripheral offers.
            let gap = (1..5).contains(&t);
            if !gap && ke < 3 && er_tx.ready().to_bool() {
                er_tx.send(beat(3, ke));
                ke += 1;
            }
            if t >= 1 && k0 < 3 && r0_tx.ready().to_bool() {
                r0_tx.send(beat(1, k0));
                k0 += 1;
            }
            if let Some(x) = r_rx.recv() {
                got.push(x.id.raw() as u8);
            }
            sim.cycle();
        }
        assert_eq!(got, vec![3, 3, 3, 1, 1, 1]);
    }

    /// The answer half's write responses: the request half's own goes
    /// first, then the lowest peripheral's, and none is lost while the
    /// host takes nothing.
    #[test]
    fn the_answer_half_orders_the_write_responses() {
        let (b0_tx, b0_rx) = chan::<B<3>, DefaultClock>();
        let (b1_tx, b1_rx) = chan::<B<3>, DefaultClock>();
        let (_r0_tx, r0_rx) = chan::<R<32, 3>, DefaultClock>();
        let (_r1_tx, r1_rx) = chan::<R<32, 3>, DefaultClock>();
        let (eb_tx, eb_rx) = chan::<B<3>, DefaultClock>();
        let (_er_tx, er_rx) = chan::<R<32, 3>, DefaultClock>();
        let (b_tx, b_rx) = chan::<B<3>, DefaultClock>();
        let (r_tx, _r_rx) = chan::<R<32, 3>, DefaultClock>();
        let mut rsp = Rsp2::default();
        let mut sim =
            Running::new(rsp.run(
                ([b0_rx, b1_rx], [r0_rx, r1_rx], eb_rx, er_rx),
                (b_tx, r_tx),
            ));
        let resp = |id: u8, resp: Resp| B {
            id: U::<3>::from(id),
            resp,
        };
        // All three offer in the same cycle, the highest peripheral's
        // written first.
        b1_tx.send(resp(2, Resp::Okay));
        b0_tx.send(resp(1, Resp::Okay));
        eb_tx.send(resp(3, Resp::DecErr));
        let mut got = Vec::new();
        for t in 0..16 {
            // The host takes nothing in cycles 2 to 5.
            if !(2..6).contains(&t) {
                if let Some(x) = b_rx.recv() {
                    got.push((x.id.raw() as u8, x.resp));
                }
            }
            sim.cycle();
        }
        assert_eq!(
            got,
            vec![(3, Resp::DecErr), (1, Resp::Okay), (2, Resp::Okay)]
        );
    }

    /// The request half alone, its ports driven by hand, and one unit
    /// of it with its channels' ends.
    #[allow(clippy::type_complexity)]
    fn request_half() -> (
        Tx<Aw<16, 3>>,
        Tx<Ar<16, 3>>,
        Tx<W<32, 4>>,
        [Rx<Aw<16, 3>>; 2],
        [Rx<Ar<16, 3>>; 2],
        [Rx<W<32, 4>>; 2],
        Rx<B<3>>,
        Rx<R<32, 3>>,
        impl std::future::Future<Output = ()>,
    ) {
        let (aw_tx, aw_rx) = chan::<Aw<16, 3>, DefaultClock>();
        let (ar_tx, ar_rx) = chan::<Ar<16, 3>, DefaultClock>();
        let (w_tx, w_rx) = chan::<W<32, 4>, DefaultClock>();
        let (aw0_tx, aw0_rx) = chan::<Aw<16, 3>, DefaultClock>();
        let (aw1_tx, aw1_rx) = chan::<Aw<16, 3>, DefaultClock>();
        let (ar0_tx, ar0_rx) = chan::<Ar<16, 3>, DefaultClock>();
        let (ar1_tx, ar1_rx) = chan::<Ar<16, 3>, DefaultClock>();
        let (w0_tx, w0_rx) = chan::<W<32, 4>, DefaultClock>();
        let (w1_tx, w1_rx) = chan::<W<32, 4>, DefaultClock>();
        let (eb_tx, eb_rx) = chan::<B<3>, DefaultClock>();
        let (er_tx, er_rx) = chan::<R<32, 3>, DefaultClock>();
        let run = async move {
            let mut req = Req2::default();
            req.run(
                (aw_rx, ar_rx, w_rx),
                (
                    [aw0_tx, aw1_tx],
                    [ar0_tx, ar1_tx],
                    [w0_tx, w1_tx],
                    eb_tx,
                    er_tx,
                ),
            )
            .await
        };
        (
            aw_tx,
            ar_tx,
            w_tx,
            [aw0_rx, aw1_rx],
            [ar0_rx, ar1_rx],
            [w0_rx, w1_rx],
            eb_rx,
            er_rx,
            run,
        )
    }

    /// Two reads of a hole, one behind the other, with nobody taking
    /// the answers at first: the request half takes the second only
    /// when the first one's beats are on their way, holds the second
    /// one's while its link is full, and the beats come out whole and
    /// in order, each burst ended by its last beat.
    #[test]
    fn the_request_half_answers_two_holes_in_order() {
        let (_aw, ar, _w, _aws, ars, _ws, _eb, er, run) = request_half();
        let mut sim = Running::new(Box::pin(run));
        let hole = |id: u8| Ar {
            id: U::<3>::from(id),
            addr: U::<16>::from(0x7000u32),
            len: U::<8>::from(1u8),
            ..Default::default()
        };
        let mut sent = 0u8;
        let mut got = Vec::new();
        for t in 0..40 {
            if sent < 2 && ar.ready().to_bool() {
                ar.send(hole(sent + 1));
                sent += 1;
            }
            // Nobody takes an answer in the first sixteen cycles.
            if t >= 16 {
                if let Some(x) = er.recv() {
                    got.push((x.id.raw() as u8, x.last.to_bool(), x.resp));
                }
            }
            assert!(ars[0].recv().is_none(), "nothing for a peripheral");
            assert!(ars[1].recv().is_none(), "nothing for a peripheral");
            sim.cycle();
        }
        assert_eq!(
            got,
            vec![
                (1, false, Resp::DecErr),
                (1, true, Resp::DecErr),
                (2, false, Resp::DecErr),
                (2, true, Resp::DecErr),
            ]
        );
    }

    /// Writes to a hole whose answers nobody takes: the third one's
    /// answer finds its link full and stays owed. While it is owed, a
    /// write to a peripheral still goes through, and a fourth write to
    /// a hole waits. When the answers are taken, all four come, in
    /// order.
    #[test]
    fn a_peripherals_write_passes_while_a_holes_answer_is_owed() {
        let (aw, _ar, w, aws, _ars, ws, eb, _er, run) = request_half();
        let mut sim = Running::new(Box::pin(run));
        let phase = |id: u8, addr: u32| Aw {
            id: U::<3>::from(id),
            addr: U::<16>::from(addr),
            ..Default::default()
        };
        let beat = W {
            data: U::<32>::from(9u32),
            strb: U::<4>::from(0xfu8),
            last: Bit::One,
        };
        // Three to a hole, one to the first peripheral, one to a hole.
        let plan = [
            (1u8, 0x7000u32),
            (2, 0x7000),
            (3, 0x7000),
            (4, 0x1000),
            (5, 0x7000),
        ];
        let (mut ia, mut iw) = (0usize, 0usize);
        let mut to_per = Vec::new();
        let mut beats_to_per = 0usize;
        let mut answers = Vec::new();
        for t in 0..80 {
            if ia < plan.len() && aw.ready().to_bool() {
                aw.send(phase(plan[ia].0, plan[ia].1));
                ia += 1;
            }
            if iw < plan.len() && w.ready().to_bool() {
                w.send(beat);
                iw += 1;
            }
            if let Some(x) = aws[0].recv() {
                to_per.push(x.id.raw() as u8);
            }
            if ws[0].recv().is_some() {
                beats_to_per += 1;
            }
            assert!(aws[1].recv().is_none(), "nothing for the other one");
            if t == 30 {
                // Nobody has taken an answer yet: two are in the link,
                // the third is owed, and the fifth write waits.
                assert_eq!(to_per, vec![4], "the peripheral's write passed");
                assert_eq!(beats_to_per, 1, "with its beat");
                assert_eq!(ia, 5, "every address phase was offered");
            }
            if t >= 30 {
                if let Some(x) = eb.recv() {
                    answers.push((x.id.raw() as u8, x.resp));
                }
            }
            sim.cycle();
        }
        assert_eq!(
            answers,
            vec![
                (1, Resp::DecErr),
                (2, Resp::DecErr),
                (3, Resp::DecErr),
                (5, Resp::DecErr),
            ]
        );
        assert_eq!(to_per, vec![4]);
        assert_eq!(beats_to_per, 1);
    }

    /// How the path of `a_read_across` is built.
    #[derive(Clone, Copy, PartialEq)]
    enum Shape {
        /// `Arbiter`, `Router` and `AxiPer`, every channel registered.
        Whole,
        /// Their halves, every channel registered.
        Halves,
        /// Their halves, with every channel between them unregistered,
        /// both ways.
        Unregistered,
    }

    /// One host of two on an arbiter, a router and two peripherals: a
    /// write and then a read of the first peripheral, and the cycles
    /// the read took from its issue to its answer.
    fn a_read_across(shape: Shape) -> u64 {
        let u = shape == Shape::Unregistered;
        // The host's tracker is one unit, so its answers come to it
        // over registered channels in every shape.
        let Link {
            host,
            host_in,
            host_out,
            ..
        } = axi::<16, 32, 4, 2, 4>();
        let (haw_tx, haw_rx) = ch!(u, Aw<16, 2>);
        let (har_tx, har_rx) = ch!(u, Ar<16, 2>);
        let (hw_tx, hw_rx) = ch!(u, W<32, 4>);
        let (hb_tx, hb_rx) = chan::<B<2>, DefaultClock>();
        let (hr_tx, hr_rx) = chan::<R<32, 2>, DefaultClock>();
        // The second host, which asks for nothing.
        let (_iaw_tx, iaw_rx) = chan::<Aw<16, 2>, DefaultClock>();
        let (_iar_tx, iar_rx) = chan::<Ar<16, 2>, DefaultClock>();
        let (_iw_tx, iw_rx) = chan::<W<32, 4>, DefaultClock>();
        let (ib_tx, _ib_rx) = chan::<B<2>, DefaultClock>();
        let (ir_tx, _ir_rx) = chan::<R<32, 2>, DefaultClock>();
        // Arbiter to router.
        let (xaw_tx, xaw_rx) = ch!(u, Aw<16, 3>);
        let (xar_tx, xar_rx) = ch!(u, Ar<16, 3>);
        let (xw_tx, xw_rx) = ch!(u, W<32, 4>);
        let (xb_tx, xb_rx) = ch!(u, B<3>);
        let (xr_tx, xr_rx) = ch!(u, R<32, 3>);
        // Router to each peripheral's tracker.
        let (aw0_tx, aw0_rx) = ch!(u, Aw<16, 3>);
        let (ar0_tx, ar0_rx) = ch!(u, Ar<16, 3>);
        let (w0_tx, w0_rx) = ch!(u, W<32, 4>);
        let (b0_tx, b0_rx) = ch!(u, B<3>);
        let (r0_tx, r0_rx) = ch!(u, R<32, 3>);
        let (aw1_tx, aw1_rx) = ch!(u, Aw<16, 3>);
        let (ar1_tx, ar1_rx) = ch!(u, Ar<16, 3>);
        let (w1_tx, w1_rx) = ch!(u, W<32, 4>);
        let (b1_tx, b1_rx) = ch!(u, B<3>);
        let (r1_tx, r1_rx) = ch!(u, R<32, 3>);
        let (eb_tx, eb_rx) = ch!(u, B<3>);
        let (er_tx, er_rx) = ch!(u, R<32, 3>);
        // Each tracker and its client.
        let Link {
            per: p0,
            per_in: pin0,
            per_out: pout0,
            ..
        } = axi::<16, 32, 4, 3, 8>();
        let Link {
            per: p1,
            per_in: pin1,
            per_out: pout1,
            ..
        } = axi::<16, 32, 4, 3, 8>();
        let (s0, s1) = (Seen::default(), Seen::default());
        let took = Rc::new(RefCell::new(0u64));
        let rec = took.clone();
        let client = async move {
            let w = host.write(Wr::at(0x1000u32), &[U::from(7u32)]).await;
            assert_eq!(w.done().await.resp, Resp::Okay);
            let t0 = now();
            let p = host.read(Rd::at(0x1004u32, 1)).await;
            let r = p.done().await;
            *rec.borrow_mut() = (now() - t0) / DefaultClock::PERIOD;
            assert_eq!(r.resp, Resp::Okay);
            assert_eq!(r.data[0].raw(), 0x10004);
        };
        let servers =
            join2(peripheral(1, p0, s0.clone()), peripheral(2, p1, s1.clone()));
        let mut hu = HostA::default();
        let host_run = hu.run(
            (host_in.0, host_in.1, hb_rx, hr_rx, host_in.4),
            (haw_tx, har_tx, hw_tx, host_out.3, host_out.4, host_out.5),
        );
        let aws = [haw_rx, iaw_rx];
        let ars = [har_rx, iar_rx];
        let ws = [hw_rx, iw_rx];
        let bs = [hb_tx, ib_tx];
        let rs = [hr_tx, ir_tx];
        if shape == Shape::Whole {
            let mut arb = Arb2::default();
            let mut rtr = Rtr::default();
            let mut pu0 = Per2::default();
            let mut pu1 = Per2::default();
            let hardware = join2(
                join2(
                    host_run,
                    arb.run(
                        (aws, ars, ws, xb_rx, xr_rx),
                        (xaw_tx, xar_tx, xw_tx, bs, rs),
                    ),
                ),
                join2(
                    rtr.run(
                        (xaw_rx, xar_rx, xw_rx, [b0_rx, b1_rx], [r0_rx, r1_rx]),
                        (
                            [aw0_tx, aw1_tx],
                            [ar0_tx, ar1_tx],
                            [w0_tx, w1_tx],
                            xb_tx,
                            xr_tx,
                        ),
                    ),
                    join2(
                        pu0.run(
                            (aw0_rx, ar0_rx, w0_rx, pin0.3, pin0.4),
                            (pout0.0, pout0.1, b0_tx, r0_tx),
                        ),
                        pu1.run(
                            (aw1_rx, ar1_rx, w1_rx, pin1.3, pin1.4),
                            (pout1.0, pout1.1, b1_tx, r1_tx),
                        ),
                    ),
                ),
            );
            let mut sim = Running::new(join2(hardware, join2(client, servers)));
            for _ in 0..120 {
                sim.cycle();
            }
            drop((eb_tx, eb_rx, er_tx, er_rx));
        } else {
            // Each sender before its receiver: the request halves on
            // the way down, the answer halves on the way back.
            let mut areq = ArbReq2::default();
            let mut arsp = ArbRsp2::default();
            let mut rreq = Req2::default();
            let mut rrsp = Rsp2::default();
            let mut q0 = PerReq2::default();
            let mut q1 = PerReq2::default();
            let mut a0 = PerRsp2::default();
            let mut a1 = PerRsp2::default();
            let down = join2(
                join2(
                    host_run,
                    areq.run((aws, ars, ws), (xaw_tx, xar_tx, xw_tx)),
                ),
                join2(
                    rreq.run(
                        (xaw_rx, xar_rx, xw_rx),
                        (
                            [aw0_tx, aw1_tx],
                            [ar0_tx, ar1_tx],
                            [w0_tx, w1_tx],
                            eb_tx,
                            er_tx,
                        ),
                    ),
                    join2(
                        q0.run((aw0_rx, ar0_rx, w0_rx), (pout0.0, pout0.1)),
                        q1.run((aw1_rx, ar1_rx, w1_rx), (pout1.0, pout1.1)),
                    ),
                ),
            );
            let back = join2(
                join2(
                    a0.run((pin0.3, pin0.4), (b0_tx, r0_tx)),
                    a1.run((pin1.3, pin1.4), (b1_tx, r1_tx)),
                ),
                join2(
                    rrsp.run(
                        ([b0_rx, b1_rx], [r0_rx, r1_rx], eb_rx, er_rx),
                        (xb_tx, xr_tx),
                    ),
                    arsp.run((xb_rx, xr_rx), (bs, rs)),
                ),
            );
            let mut sim =
                Running::new(join2(down, join2(join2(client, servers), back)));
            for _ in 0..120 {
                sim.cycle();
            }
        }
        assert_eq!(s0.0.borrow().clone(), vec![(0x1000, vec![7])]);
        assert!(s1.0.borrow().is_empty(), "nothing for the other one");
        let cycles = *took.borrow();
        assert!(cycles > 0, "the read was answered");
        cycles
    }

    /// The reason for the halves. With the units whole a read crosses
    /// eight registered channels on its way and back. With the halves
    /// joined in the order of the path, every channel between them can
    /// be unregistered, both ways, which the whole units cannot be
    /// joined for; five of the eight then cost no cycle. The channels
    /// into the host's tracker and into the client stay registered.
    #[test]
    fn a_read_crosses_the_halves_unregistered_both_ways() {
        let whole = a_read_across(Shape::Whole);
        let halves = a_read_across(Shape::Halves);
        let fast = a_read_across(Shape::Unregistered);
        assert_eq!(halves, whole, "registered, the halves cost the same");
        assert_eq!(fast + 5, whole, "three hops down, two back");
    }
}
