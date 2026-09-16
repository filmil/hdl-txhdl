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
//! its data together. That is what the two hosts in this tree make,
//! and it is what keeps two of them from interleaving their write
//! data at a peripheral that has one port: AXI4 puts no identifier on
//! the write data channel, so a beat that arrived between another
//! host's address phase and its beat could not be told apart. A burst
//! of more than one beat wants a virtual channel per source, or
//! reassembly at the peripheral, and this bridge carries none.
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
/// It holds nothing: a write is one packet, so there is no burst to
/// keep track of between cycles.
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
> {}
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
            // The two address phases, and the node each is for. The
            // last range matches whatever the first two did not.
            let ah = aw.head();
            let rh = ar.head();
            let m0 = U::<A>::from(M0 as u32);
            let c0 = U::<A>::from(B0 as u32);
            let m1 = U::<A>::from(M1 as u32);
            let c1 = U::<A>::from(B1 as u32);
            let aw0 = (ah.addr & m0) == c0;
            let aw1 = (ah.addr & m1) == c1;
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
            // A write goes when its address phase and its beat are
            // both there, as one packet; a read goes on its own.
            let wh = w.head();
            let go_w = aw.peek().is_some() & w.peek().is_some() & room;
            let go_ar = !go_w & ar.peek().is_some() & room;
            let _ = aw.recv_if(go_w);
            let _ = w.recv_if(go_w);
            let _ = ar.recv_if(go_ar);
            // The answers, unpacked back into the two channels the
            // host's tracker reads.
            let ph = rsp.head();
            let is_b = ph.chan == Chan::B;
            let to_b = rsp.peek().is_some() & is_b & b.ready();
            let to_r = rsp.peek().is_some() & !is_b & r.ready();
            let _ = rsp.recv_if(to_b | to_r);
            // One packet leaves, so its fields are chosen once: a
            // write beat's when a beat is going, else the address
            // phase's, and of the two phases the read's when it is
            // the one being sent.
            let go = go_ar | go_w;
            if go.to_bool() {
                req.send(Pkt {
                    dx: mux(go_ar, arx, awx),
                    dy: mux(go_ar, ary, awy),
                    sx: me_x,
                    sy: me_y,
                    chan: mux(go_ar, Chan::Ar, Chan::W),
                    id: mux(go_ar, rh.id, ah.id),
                    addr: mux(go_ar, rh.addr, ah.addr),
                    len: mux(go_ar, rh.len, ah.len),
                    size: mux(go_ar, rh.size, ah.size),
                    burst: mux(go_ar, rh.burst, ah.burst),
                    lock: mux(go_ar, rh.lock, ah.lock),
                    cache: mux(go_ar, rh.cache, ah.cache),
                    prot: mux(go_ar, rh.prot, ah.prot),
                    qos: mux(go_ar, rh.qos, ah.qos),
                    region: mux(go_ar, rh.region, ah.region),
                    data: wh.data,
                    strb: wh.strb,
                    last: Bit::One,
                    resp: Resp::Okay,
                });
            }
            if to_b.to_bool() {
                b.send(B {
                    id: ph.id,
                    resp: ph.resp,
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
