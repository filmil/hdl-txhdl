// SPDX-License-Identifier: Apache-2.0
//! The exit: AXI on one side and packets on the other.
//!
//! [`HostBridge`] stands where an AXI host's five channels would go to
//! a peripheral, and sends them across the network instead. It packs
//! an address phase into a packet addressed by a map of three ranges,
//! packs the write beats that follow to the same node, and unpacks
//! the responses that come back. [`PerBridge`] is the other end: it
//! unpacks a request into the five channels a peripheral's tracker
//! reads, and packs the answers back to the node the request came
//! from, which the packet carries so that nothing has to keep a table
//! of who asked.
//!
//! The map's last entry is the default route: give it a mask of zero
//! and every address that matched nothing else goes there. A design
//! states its map that way and there is no such thing as an address
//! the network does not know, which is why neither bridge has to
//! answer for one.
//!
//! A write of one beat is one packet, carrying its address phase and
//! its data together, and a write of more is that many packets: the
//! host bridge splits a burst into single-beat writes, each at its own
//! address, and answers the host once, when the last of them has been
//! answered. That is what keeps two hosts from interleaving their
//! write data at a peripheral that has one port: AXI4 puts no
//! identifier on the write data channel, so a beat that arrived
//! between another host's address phase and its beat could not be
//! told apart, but a packet that is a whole write cannot be cut into.
//! What a peripheral sees is `N` writes rather than one burst of `N`,
//! which is the same thing to a memory and to every peripheral in this
//! tree; `docs/noc-bursts.md` says what it would cost to do otherwise,
//! and for whom.
use txhdl::comp::{mux, Clock, DefaultClock, Mem, Reg, Rx, Tx, Unit};
use txhdl::types::{Bit, U};
use txhdl::{lower, with, Trace};

use super::pkt::{Chan, Pkt};
use crate::bus::axi::{Addr, BurstKind, Resp, B, R, W};

// begin{host}
/// The host side of an exit, at column `X` and row `Y`.
///
/// `B0`, `M0`, `X0` and `Y0`, and their two fellows, are the address
/// map: an address whose bits under the mask `M0` equal `B0` goes to
/// the node at `X0`, `Y0`. The last entry is the default route, so a
/// mask of zero there takes everything that matched nothing else.
///
/// The widths are the AXI link's, and they are the same everywhere in
/// the network: `A` is the address width, `D` the data width, `S` the
/// strobe width, which is `D / 8`, and `I` the identifier width. `XB`
/// and `YB` are the widths of a coordinate, so a lattice is `1 << XB`
/// by `1 << YB` nodes at most.
///
/// A write of one beat is one packet and needs nothing kept between
/// cycles. A write of more is split (issue 125): its first beat leaves
/// with the address phase, and each beat after it leaves as a
/// single-beat write of its own at the next address, under the phase
/// held here. Every one of those is answered by the far side, and the
/// bridge answers the host once, when the last answer is in, with
/// `SlvErr` if any of them was an error. One burst is split at a
/// time, and the next long write waits until this one is answered; a
/// write of one beat and a read do not wait. A wrapping burst is not
/// split, since the wrap is not computed here: it is eaten whole and
/// answered `SlvErr`, as every long write was before the split.
#[derive(Trace, Default)]
pub struct HostBridge<
    const X: usize,
    const Y: usize,
    const XB: usize,
    const YB: usize,
    const A: usize,
    const D: usize,
    const S: usize,
    const I: usize,
    const B0: usize,
    const M0: usize,
    const X0: usize,
    const Y0: usize,
    const B1: usize,
    const M1: usize,
    const X1: usize,
    const Y1: usize,
    const B2: usize,
    const M2: usize,
    const X2: usize,
    const Y2: usize,
> {
    /// Beats of the burst being split that are still to leave, after
    /// the one leaving now; zero when no burst is being split.
    left: Reg<U<8>>,
    /// The address the next beat of that burst goes to.
    saddr: Reg<U<A>>,
    /// The address phase held for the beats after the first: its
    /// identifier, which the merged answer also carries; whether it
    /// is fixed, in which case the address does not move; and its
    /// size and the hints AXI carries and nothing here acts on, sent
    /// again with every beat so that the far side sees the same phase
    /// each time.
    sid: Reg<U<I>>,
    ssize: Reg<U<3>>,
    sfix: Reg<Bit>,
    slock: Reg<Bit>,
    scache: Reg<U<4>>,
    sprot: Reg<U<3>>,
    sqos: Reg<U<4>>,
    sregion: Reg<U<4>>,
    /// Answers still to come for the split burst before the host is
    /// answered; zero when none is owed.
    pend: Reg<U<8>>,
    /// Whether any of the answers in so far was an error, which makes
    /// the merged answer `SlvErr`.
    err: Reg<Bit>,
    /// The beats of a refused burst are still to come, so every one
    /// of them is taken and dropped until the one marked last.
    eat: Reg<Bit>,
    /// A refused burst has been eaten whole and owes its host an
    /// answer, which goes out as soon as the response channel is free.
    owe: Reg<Bit>,
    /// The identifier that answer carries, kept from the address
    /// phase that was refused.
    bad: Reg<U<I>>,
}
// end{host}

// begin{hostrun}
#[lower]
impl<
        const X: usize,
        const Y: usize,
        const XB: usize,
        const YB: usize,
        const A: usize,
        const D: usize,
        const S: usize,
        const I: usize,
        const B0: usize,
        const M0: usize,
        const X0: usize,
        const Y0: usize,
        const B1: usize,
        const M1: usize,
        const X1: usize,
        const Y1: usize,
        const B2: usize,
        const M2: usize,
        const X2: usize,
        const Y2: usize,
    > Unit
    for HostBridge<
        X,
        Y,
        XB,
        YB,
        A,
        D,
        S,
        I,
        B0,
        M0,
        X0,
        Y0,
        B1,
        M1,
        X1,
        Y1,
        B2,
        M2,
        X2,
        Y2,
    >
{
    async fn run(
        &mut self,
        (aw, ar, w, rsp): (
            Rx<Addr<A, I>>,
            Rx<Addr<A, I>>,
            Rx<W<D, S>>,
            Rx<Pkt<XB, YB, A, D, S, I>>,
        ),
        (req, b, r): (Tx<Pkt<XB, YB, A, D, S, I>>, Tx<B<I>>, Tx<R<D, I>>),
    ) {
        loop {
            DefaultClock::rising().await;
            let me_x = U::<XB>::from(X as u32);
            let me_y = U::<YB>::from(Y as u32);
            let room = req.ready();
            let ah = aw.head();
            let rh = ar.head();
            let wh = w.head();
            // A burst being split: its beats after the first leave
            // under the phase held in the registers, at the address
            // held there, so the address a write packet is for is the
            // held one while splitting and the offered phase's else.
            let none = U::<8>::from(0u8);
            let splitting = self.left.get() != none;
            let merging = self.pend.get() != none;
            let eating = self.eat.get();
            let offered = aw.peek().is_some();
            let long = ah.len != none;
            let wrapping = ah.burst == BurstKind::Wrap;
            let addr = mux(splitting, self.saddr.get(), ah.addr);
            // The node each address is for. The last range matches
            // whatever the first two did not.
            let m0 = U::<A>::from(M0 as u32);
            let c0 = U::<A>::from(B0 as u32);
            let m1 = U::<A>::from(M1 as u32);
            let c1 = U::<A>::from(B1 as u32);
            let aw0 = (addr & m0) == c0;
            let aw1 = (addr & m1) == c1;
            let ar0 = (rh.addr & m0) == c0;
            let ar1 = (rh.addr & m1) == c1;
            let awx = mux(
                aw0,
                U::<XB>::from(X0 as u32),
                mux(aw1, U::<XB>::from(X1 as u32), U::<XB>::from(X2 as u32)),
            );
            let awy = mux(
                aw0,
                U::<YB>::from(Y0 as u32),
                mux(aw1, U::<YB>::from(Y1 as u32), U::<YB>::from(Y2 as u32)),
            );
            let arx = mux(
                ar0,
                U::<XB>::from(X0 as u32),
                mux(ar1, U::<XB>::from(X1 as u32), U::<XB>::from(X2 as u32)),
            );
            let ary = mux(
                ar0,
                U::<YB>::from(Y0 as u32),
                mux(ar1, U::<YB>::from(Y1 as u32), U::<YB>::from(Y2 as u32)),
            );
            // A wrapping burst is refused rather than split, since the
            // wrap is not computed here: its address phase and every
            // one of its beats are taken and dropped, and its host is
            // answered `SlvErr`, because a peripheral that never sees
            // the burst never answers it and the host would wait for
            // ever.
            let refuse = !eating & !splitting & offered & wrapping;
            // A write's first beat goes when its address phase and
            // its beat are both there, as one packet; a long write's
            // first beat waits too until the last long write has been
            // answered, since one answer is merged at a time. The
            // beats after the first go as the channel offers them. A
            // read goes on its own.
            let first = !eating
                & !splitting
                & !refuse
                & offered
                & w.peek().is_some()
                & room
                & (!long | !merging);
            let more = splitting & w.peek().is_some() & room;
            let go_w = first | more;
            let go_ar = !go_w & ar.peek().is_some() & room;
            let drop_w = (eating | refuse) & w.peek().is_some();
            let ate_last = drop_w & wh.last;
            let _ = aw.recv_if(first | refuse);
            let _ = w.recv_if(go_w | drop_w);
            let _ = ar.recv_if(go_ar);
            // The phase a write packet carries: the held one while
            // splitting, the offered one for a first beat. The address
            // moves by a beat, which is the link's width: every beat in
            // this tree is a whole word, the memory model steps by the
            // width, and `size` is carried and not acted on anywhere
            // (issue 374). A fixed burst does not move at all.
            let id = mux(splitting, self.sid.get(), ah.id);
            let size = mux(splitting, self.ssize.get(), ah.size);
            let is_fixed = Bit::from(ah.burst == BurstKind::Fixed);
            let fixed = mux(splitting, self.sfix.get(), is_fixed);
            let bytes = U::<A>::from(S as u32);
            let step = mux(fixed, U::<A>::from(0u8), bytes);
            let start = first & long;
            // The answers, unpacked back into the two channels the
            // host's tracker reads. A write response for the burst
            // being merged is counted rather than passed on, and the
            // one that completes the count is answered to the host in
            // its place, so it waits for room on `b` as a passed-on
            // one does.
            let ph = rsp.head();
            let got = rsp.peek().is_some();
            let is_b = ph.chan == Chan::B;
            let mine = is_b & merging & (ph.id == self.sid.get());
            let last_one = self.pend.get() == U::<8>::from(1u8);
            let done_b = got & mine & last_one & b.ready();
            let merge = got & mine & (!last_one | b.ready());
            let to_b = got & is_b & !mine & b.ready();
            let to_r = got & !is_b & r.ready();
            let _ = rsp.recv_if(to_b | merge | to_r);
            let bad_resp =
                !(ph.resp == Resp::Okay) & !(ph.resp == Resp::ExOkay);
            let err_now = self.err.get() | (merge & bad_resp);
            // The refusal's own answer, which waits behind whatever
            // the network is answering rather than racing it.
            let owing = self.owe.get();
            let say = owing & !to_b & !done_b & b.ready();
            with!(self <= {
                eat: mux(ate_last, Bit::Zero, mux(refuse, Bit::One, eating)),
                bad: mux(refuse, ah.id, self.bad.get()),
                owe: mux(say, Bit::Zero, mux(ate_last, Bit::One, owing)),
                first ? {
                    left: ah.len,
                    sid: ah.id,
                    ssize: ah.size,
                    sfix: is_fixed,
                    slock: ah.lock,
                    scache: ah.cache,
                    sprot: ah.prot,
                    sqos: ah.qos,
                    sregion: ah.region,
                } else {
                    more ? left: self.left.get() - 1,
                },
                go_w ? saddr: addr + step,
                start ? {
                    pend: ah.len + 1,
                    err: Bit::Zero,
                } else {
                    merge ? {
                        pend: self.pend.get() - 1,
                        err: err_now,
                    },
                },
            });
            // One packet leaves, so its fields are chosen once: a
            // write beat's when a beat is going, else the address
            // phase's, and of the two phases the read's when it is
            // the one being sent. A write packet is a single-beat
            // write whatever the burst was, so its length is zero and
            // it is marked last.
            let go = go_ar | go_w;
            if go.to_bool() {
                req.send(Pkt {
                    dx: mux(go_ar, arx, awx),
                    dy: mux(go_ar, ary, awy),
                    sx: me_x,
                    sy: me_y,
                    chan: mux(go_ar, Chan::Ar, Chan::W),
                    id: mux(go_ar, rh.id, id),
                    addr: mux(go_ar, rh.addr, addr),
                    len: mux(go_ar, rh.len, U::<8>::from(0u8)),
                    size: mux(go_ar, rh.size, size),
                    burst: mux(go_ar, rh.burst, BurstKind::Incr),
                    lock: mux(
                        go_ar,
                        rh.lock,
                        mux(splitting, self.slock.get(), ah.lock),
                    ),
                    cache: mux(
                        go_ar,
                        rh.cache,
                        mux(splitting, self.scache.get(), ah.cache),
                    ),
                    prot: mux(
                        go_ar,
                        rh.prot,
                        mux(splitting, self.sprot.get(), ah.prot),
                    ),
                    qos: mux(
                        go_ar,
                        rh.qos,
                        mux(splitting, self.sqos.get(), ah.qos),
                    ),
                    region: mux(
                        go_ar,
                        rh.region,
                        mux(splitting, self.sregion.get(), ah.region),
                    ),
                    data: wh.data,
                    strb: wh.strb,
                    last: Bit::One,
                    resp: Resp::Okay,
                });
            }
            // One send on the response channel, whichever answer it
            // is: the network's passed on, the merged one for a split
            // burst, or the refusal's.
            if (to_b | done_b | say).to_bool() {
                b.send(B {
                    id: mux(
                        to_b,
                        ph.id,
                        mux(done_b, self.sid.get(), self.bad.get()),
                    ),
                    resp: mux(
                        to_b,
                        ph.resp,
                        mux(
                            done_b,
                            mux(err_now, Resp::SlvErr, Resp::Okay),
                            Resp::SlvErr,
                        ),
                    ),
                });
            }
            if to_r.to_bool() {
                r.send(R {
                    id: ph.id,
                    data: ph.data,
                    resp: ph.resp,
                    last: ph.last,
                });
            }
        }
    }
}
// end{hostrun}

// begin{per}
/// The peripheral side of an exit, at node `X`, `Y`. It unpacks a
/// request into the channels a peripheral's tracker reads and packs
/// the answers back to the node the request came from.
///
/// An identifier belongs to the host that made it, so two hosts may
/// use the same one and a peripheral behind one port cannot tell
/// their bursts apart. This bridge therefore gives every request an
/// identifier of its own, from `NIDS` of them, and remembers against
/// it the node the request came from and the identifier that node
/// used. The answer comes back under the local one, and the packet
/// goes home under the original. A request waits when the identifier
/// whose turn it is has not been answered, which is what bounds how
/// many bursts a peripheral has in flight.
#[derive(Trace, Default)]
pub struct PerBridge<
    const X: usize,
    const Y: usize,
    const XB: usize,
    const YB: usize,
    const A: usize,
    const D: usize,
    const S: usize,
    const I: usize,
    const NIDS: usize,
> {
    /// The column of the node each local identifier's burst came from.
    pub sx: Mem<U<XB>, NIDS>,
    /// The row of it.
    pub sy: Mem<U<YB>, NIDS>,
    /// The identifier that node used, which its answer goes back
    /// under.
    pub oid: Mem<U<I>, NIDS>,
    /// The local identifier whose turn it is to be given out.
    pub turn: Reg<U<I>>,
    /// Which local identifiers are out and not yet answered.
    pub busy: Reg<U<NIDS>>,
}
// end{per}

// begin{perrun}
#[lower]
impl<
        const X: usize,
        const Y: usize,
        const XB: usize,
        const YB: usize,
        const A: usize,
        const D: usize,
        const S: usize,
        const I: usize,
        const NIDS: usize,
    > Unit for PerBridge<X, Y, XB, YB, A, D, S, I, NIDS>
{
    async fn run(
        &mut self,
        (req, b, r): (Rx<Pkt<XB, YB, A, D, S, I>>, Rx<B<I>>, Rx<R<D, I>>),
        (aw, ar, w, rsp): (
            Tx<Addr<A, I>>,
            Tx<Addr<A, I>>,
            Tx<W<D, S>>,
            Tx<Pkt<XB, YB, A, D, S, I>>,
        ),
    ) {
        loop {
            DefaultClock::rising().await;
            let me_x = U::<XB>::from(X as u32);
            let me_y = U::<YB>::from(Y as u32);
            // A request, unpacked. A write carries its address phase
            // and its beat together, so it needs room on both.
            let ph = req.head();
            let offered = req.peek().is_some();
            let writing = ph.chan == Chan::W;
            // The identifier this request will carry here, and
            // whether it is free to be given out.
            let mine = self.turn.get();
            let free = !self.busy.get().bit(mine.raw() as usize);
            let take_w = offered & writing & free & aw.ready() & w.ready();
            let take_r = offered & !writing & free & ar.ready();
            let taken = take_w | take_r;
            let _ = req.recv_if(taken);
            // The answers. One packet a cycle, a write response
            // before a read beat.
            let bh = b.head();
            let rh = r.head();
            let go_b = b.peek().is_some() & rsp.ready();
            let go_r = r.peek().is_some() & rsp.ready() & !go_b;
            let _ = b.recv_if(go_b);
            let _ = r.recv_if(go_r);
            let back = mux(go_b, bh.id, rh.id);
            // An identifier is free again when its burst is answered:
            // a write response, or the last beat of a read.
            let ends = go_b | (go_r & rh.last);
            let one = U::<NIDS>::from(1u8);
            let zero = U::<NIDS>::from(0u8);
            let mine_bit = one << (mine.raw() as usize);
            let back_bit = one << (back.raw() as usize);
            with!(self <= {
                taken ? {
                    sx.at(mine): ph.sx,
                    sy.at(mine): ph.sy,
                    oid.at(mine): ph.id,
                    turn: mine + 1,
                },
                busy: (self.busy.get() | mux(taken, mine_bit, zero))
                    & !mux(ends, back_bit, zero),
            });
            if take_w.to_bool() {
                aw.send(Addr {
                    id: mine,
                    addr: ph.addr,
                    len: ph.len,
                    size: ph.size,
                    burst: ph.burst,
                    lock: ph.lock,
                    cache: ph.cache,
                    prot: ph.prot,
                    qos: ph.qos,
                    region: ph.region,
                });
                w.send(W {
                    data: ph.data,
                    strb: ph.strb,
                    last: Bit::One,
                });
            }
            if take_r.to_bool() {
                ar.send(Addr {
                    id: mine,
                    addr: ph.addr,
                    len: ph.len,
                    size: ph.size,
                    burst: ph.burst,
                    lock: ph.lock,
                    cache: ph.cache,
                    prot: ph.prot,
                    qos: ph.qos,
                    region: ph.region,
                });
            }
            if (go_b | go_r).to_bool() {
                rsp.send(Pkt {
                    dx: self.sx.read(back),
                    dy: self.sy.read(back),
                    sx: me_x,
                    sy: me_y,
                    chan: mux(go_b, Chan::B, Chan::R),
                    id: self.oid.read(back),
                    addr: U::<A>::from(0u8),
                    len: U::<8>::from(0u8),
                    size: U::<3>::from(0u8),
                    burst: BurstKind::Incr,
                    lock: Bit::Zero,
                    cache: U::<4>::from(0u8),
                    prot: U::<3>::from(0u8),
                    qos: U::<4>::from(0u8),
                    region: U::<4>::from(0u8),
                    data: rh.data,
                    strb: U::<S>::from(0u8),
                    last: mux(go_b, Bit::One, rh.last),
                    resp: mux(go_b, bh.resp, rh.resp),
                });
            }
        }
    }
}
// end{perrun}
