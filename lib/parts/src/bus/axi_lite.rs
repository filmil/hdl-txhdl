// SPDX-License-Identifier: Apache-2.0
//! AXI4-Lite, and the bridge that puts small peripherals behind an
//! AXI4 link.
//!
//! AXI4-Lite is AXI4 with everything a small peripheral does not need
//! taken out: no identifier, no burst, no `last`, so one transaction
//! is one beat on each channel it uses. A peripheral on it is a few
//! lines of hardware with no tracker in front of it: a read is an
//! address in and a word out, and a write is an address and a word in
//! and a response out.
//!
//! [`axi_lite`] makes the five channels of a link, as
//! [`axi_units`](super::axi::axi_units) does for AXI4, and hands back
//! the ends each side holds. [`LiteBridge`] sits between an AXI4 link
//! and `N` AXI-Lite peripherals: it takes the AXI4 side one burst at a
//! time, decodes the burst's address to a peripheral by the ranges its
//! address map states, sends each beat to it as one AXI-Lite
//! transaction, and answers the burst with its identifier. One unit
//! serves every count: its peripheral side is arrays of ports, and the
//! lowering unrolls the loops over them when `lowered` runs (issue
//! 500), where `lite_bridge!` used to write one unit per count from one
//! to eight.
//!
//! The widths are stated, as everywhere in this library: `A` the
//! address width, `D` the data width and `S` the strobe width, which
//! is `D / 8`.
use crate::bus::axi::{Ar, Aw, BurstKind, Resp, B, R, W};
use std::marker::PhantomData;
use txhdl::comp::{chan, mux, Clock, DefaultClock, Link, Reg, Rx, Tx, Unit};
use txhdl::map::AddrMap;
use txhdl::types::{Bit, U};
use txhdl::{lower, with, Trace};
use txhdl::{
    Ports as PortsDerive, Transaction as TransactionDerive,
    Value as ValueDerive,
};

// begin{beats}
/// The address phase of an AXI-Lite transaction, on either address
/// channel: the address, and the protection bits AXI4 calls `AxPROT`.
#[derive(TransactionDerive, ValueDerive, Clone, Copy, Default, Debug)]
pub struct LiteAddr<const A: usize> {
    /// The address of the word read or written.
    pub addr: U<A>,
    /// Privilege, security and whether this is an instruction fetch.
    pub prot: U<3>,
}

/// The write address channel's beat.
pub type LiteAw<const A: usize> = LiteAddr<A>;
/// The read address channel's beat. The same fields.
pub type LiteAr<const A: usize> = LiteAddr<A>;

/// The write data channel's beat: the word, and which of its bytes
/// are meant.
#[derive(TransactionDerive, ValueDerive, Clone, Copy, Default, Debug)]
pub struct LiteW<const D: usize, const S: usize> {
    /// The word written.
    pub data: U<D>,
    /// A bit per byte lane; a lane whose bit is low is not written.
    pub strb: U<S>,
}

/// The write response channel's beat: how the write went.
#[derive(TransactionDerive, ValueDerive, Clone, Copy, Default, Debug)]
pub struct LiteB {
    /// How it went.
    pub resp: Resp,
}

/// The read data channel's beat: the word read, and how the read went.
#[derive(TransactionDerive, ValueDerive, Clone, Copy, Default, Debug)]
pub struct LiteR<const D: usize> {
    /// The word read.
    pub data: U<D>,
    /// How it went.
    pub resp: Resp,
}
// end{beats}

/// The ends a host holds on an AXI-Lite link, in the order of the
/// channels: the two address channels and the write data it drives,
/// and the write response and read data it reads.
pub type LiteHost<const A: usize, const D: usize, const S: usize> = (
    Tx<LiteAw<A>>,
    Tx<LiteAr<A>>,
    Tx<LiteW<D, S>>,
    Rx<LiteB>,
    Rx<LiteR<D>>,
);

/// The ends a peripheral holds on an AXI-Lite link, in the same order:
/// what it reads, and then what it drives.
pub type LitePer<const A: usize, const D: usize, const S: usize> = (
    Rx<LiteAw<A>>,
    Rx<LiteAr<A>>,
    Rx<LiteW<D, S>>,
    Tx<LiteB>,
    Tx<LiteR<D>>,
);

/// What a peripheral holds on an AXI-Lite link, as one port rather
/// than five: `run(&mut self, bus: LitePort<32, 32, 4>, ..)` and
/// `bus.ar` in the body. Each field is one channel, its beat one of
/// the structs above, and the netlist names each port for the side and
/// the field, `bus_aw_valid`, `bus_r_data` and so on, as AXI's own
/// signals are named with a prefix. The fields are in [`LitePer`]'s
/// order. Any unit in any crate may take it, since `#[derive(Ports)]`
/// is all `#[lower]` needs of it (issue 483).
#[derive(PortsDerive)]
pub struct LitePort<const A: usize, const D: usize, const S: usize> {
    /// Write addresses, in.
    pub aw: Rx<LiteAw<A>>,
    /// Read addresses, in.
    pub ar: Rx<LiteAr<A>>,
    /// Write data, in.
    pub w: Rx<LiteW<D, S>>,
    /// Write responses, out.
    pub b: Tx<LiteB>,
    /// Read data, out.
    pub r: Tx<LiteR<D>>,
}

impl<const A: usize, const D: usize, const S: usize> From<LitePer<A, D, S>>
    for LitePort<A, D, S>
{
    fn from((aw, ar, w, b, r): LitePer<A, D, S>) -> Self {
        LitePort { aw, ar, w, b, r }
    }
}

/// What a host holds on an AXI-Lite link, as one port: [`LitePort`]'s
/// five channels by the same names, each end turned round. `link` makes
/// the two at once: `let (host, per) = link::<LitePort<32, 32, 4>>()`
/// (issue 498).
#[derive(PortsDerive)]
pub struct LiteHostPort<const A: usize, const D: usize, const S: usize> {
    /// Write addresses, out.
    pub aw: Tx<LiteAw<A>>,
    /// Read addresses, out.
    pub ar: Tx<LiteAr<A>>,
    /// Write data, out.
    pub w: Tx<LiteW<D, S>>,
    /// Write responses, in.
    pub b: Rx<LiteB>,
    /// Read data, in.
    pub r: Rx<LiteR<D>>,
}

impl<const A: usize, const D: usize, const S: usize> Link
    for LitePort<A, D, S>
{
    type Host = LiteHostPort<A, D, S>;
    fn link() -> (Self::Host, Self) {
        let LiteLink { host, per } = axi_lite::<A, D, S>();
        let (aw, ar, w, b, r) = host;
        (LiteHostPort { aw, ar, w, b, r }, per.into())
    }
}

/// An AXI-Lite link: the ends of its five channels, by side.
pub struct LiteLink<const A: usize, const D: usize, const S: usize> {
    /// What the host holds: a bridge's `aw`, `ar` and `w` it drives
    /// and `b` and `r` it reads for this peripheral.
    pub host: LiteHost<A, D, S>,
    /// What the peripheral holds.
    pub per: LitePer<A, D, S>,
}

/// Make an AXI-Lite link: its five channels, and the ends each side
/// holds, the way [`chan`] makes a channel and hands back two.
///
/// [`chan`]: txhdl::comp::chan
pub fn axi_lite<const A: usize, const D: usize, const S: usize>(
) -> LiteLink<A, D, S> {
    let (aw_tx, aw_rx) = chan::<LiteAw<A>, DefaultClock>();
    let (ar_tx, ar_rx) = chan::<LiteAr<A>, DefaultClock>();
    let (w_tx, w_rx) = chan::<LiteW<D, S>, DefaultClock>();
    let (b_tx, b_rx) = chan::<LiteB, DefaultClock>();
    let (r_tx, r_rx) = chan::<LiteR<D>, DefaultClock>();
    LiteLink {
        host: (aw_tx, ar_tx, w_tx, b_rx, r_rx),
        per: (aw_rx, ar_rx, w_rx, b_tx, r_tx),
    }
}

// begin{part}
/// An AXI4 to AXI-Lite bridge of `N` peripherals, whose address ranges
/// the map `M` states (issue 593). The five AXI4 channels of one link
/// come in; five AXI-Lite channels go out per peripheral, as arrays
/// of `N`, and a peripheral's range is its entry of `M::RANGES`,
/// matched on a burst's address, the first that matches winning. The
/// bridge takes one burst at a time, since AXI-Lite has no identifier
/// to tell two apart. Each beat of a burst is one AXI-Lite
/// transaction, at the burst's address moved on by a beat's width per
/// beat, or not moved for a fixed burst: a read's answers go up as the
/// burst's beats, the last marked last, and a write's answers are
/// folded into one response, the first error or `Okay`. A burst to no
/// peripheral's range is answered `DecErr` by the bridge itself, beat
/// by beat, so a read still gets every beat it asked for. Written in
/// the lowered subset, so it is a netlist too.
// begin{state}
#[derive(Trace)]
pub struct LiteBridge<
    const N: usize,
    M: AddrMap<N>,
    const A: usize,
    const D: usize,
    const S: usize,
    const I: usize,
> {
    /// The map, which holds no signal: a marker the netlist ignores.
    pub map: PhantomData<M>,
    /// A burst is in progress, and the next waits for it.
    pub busy: Reg<Bit>,
    /// The burst is a read.
    pub rd: Reg<Bit>,
    /// The read address channel goes first when both offer.
    pub rfirst: Reg<Bit>,
    /// The burst's identifier, which every answer to it names.
    pub xid: Reg<U<I>>,
    /// The address of the current beat.
    pub addr: Reg<U<A>>,
    /// How far the address moves per beat: none for a fixed burst.
    pub stride: Reg<U<A>>,
    /// Beats left after this one.
    pub left: Reg<U<8>>,
    /// The burst's protection bits, given with every beat.
    pub prot: Reg<U<3>>,
    /// Which peripheral the burst decoded to, one bit each; none set
    /// is a hole, answered here.
    pub sel: Reg<U<N>>,
    /// The beat's AXI-Lite request is out, and its answer awaited.
    pub sent: Reg<Bit>,
    /// A write burst's response so far: its first error, or `Okay`.
    pub wresp: Reg<Resp>,
}
// end{state}

impl<
        const N: usize,
        M: AddrMap<N>,
        const A: usize,
        const D: usize,
        const S: usize,
        const I: usize,
    > Default for LiteBridge<N, M, A, D, S, I>
{
    fn default() -> Self {
        LiteBridge {
            map: PhantomData,
            busy: Reg::default(),
            rd: Reg::default(),
            rfirst: Reg::default(),
            xid: Reg::default(),
            addr: Reg::default(),
            stride: Reg::default(),
            left: Reg::default(),
            prot: Reg::default(),
            sel: Reg::default(),
            sent: Reg::default(),
            wresp: Reg::default(),
        }
    }
}

// The lowering reads a loop over an array of ports as `bs[i]` and a
// map's range as `M::RANGES[i]`, so the index is what it is written
// with, and Clippy would rather it were an iterator.
#[allow(clippy::needless_range_loop)]
// begin{ports}
#[lower]
impl<
        const N: usize,
        M: AddrMap<N>,
        const A: usize,
        const D: usize,
        const S: usize,
        const I: usize,
    > Unit for LiteBridge<N, M, A, D, S, I>
{
    async fn run(
        &mut self,
        (aw, ar, w, bs, rs): (
            Rx<Aw<A, I>>,
            Rx<Ar<A, I>>,
            Rx<W<D, S>>,
            [Rx<LiteB>; N],
            [Rx<LiteR<D>>; N],
        ),
        (aws, ars, ws, b, r): (
            [Tx<LiteAw<A>>; N],
            [Tx<LiteAr<A>>; N],
            [Tx<LiteW<D, S>>; N],
            Tx<B<I>>,
            Tx<R<D, I>>,
        ),
    ) {
        loop {
            DefaultClock::rising().await;
            // end{ports}
            // begin{accept}
            // A burst is taken when none is in progress, the two
            // address channels taking turns when both offer.
            let idle = !self.busy;
            let aw_off = aw.peek().is_some();
            let ar_off = ar.peek().is_some();
            let awh = aw.head();
            let arh = ar.head();
            let pick_ar = idle & (self.rfirst | !aw_off);
            let take_ar = pick_ar & ar_off;
            let take_aw = idle & aw_off & !take_ar;
            let _ = ar.recv_if(pick_ar);
            let _ = aw.recv_if(idle & !take_ar);
            // Where it goes: a bit per peripheral, the first range
            // that matches winning, and none for a hole.
            let mut ar_sel = U::<N>::from(0u8);
            let mut ar_any = Bit::Zero;
            let mut aw_sel = U::<N>::from(0u8);
            let mut aw_any = Bit::Zero;
            for i in 0..N {
                let one = U::<N>::from(1u8) << i;
                let ar_hit = Bit::from(
                    (arh.addr.raw() as usize & M::RANGES[i].1)
                        == M::RANGES[i].0,
                ) & !ar_any;
                ar_sel = mux(ar_hit, one, ar_sel);
                ar_any = ar_any | ar_hit;
                let aw_hit = Bit::from(
                    (awh.addr.raw() as usize & M::RANGES[i].1)
                        == M::RANGES[i].0,
                ) & !aw_any;
                aw_sel = mux(aw_hit, one, aw_sel);
                aw_any = aw_any | aw_hit;
            }
            let new_sel = mux(take_ar, ar_sel, aw_sel);
            // How far the address moves per beat: a beat's width, or
            // nothing for a fixed burst.
            let size = mux(take_ar, arh.size, awh.size);
            let kind = mux(take_ar, arh.burst, awh.burst);
            let fixed = kind == BurstKind::Fixed;
            let width = U::<A>::from(1u8) << (size.raw() as usize);
            let new_stride = mux(fixed, U::<A>::from(0u8), width);
            // end{accept}
            // begin{requests}
            // A beat's request, to the peripheral the burst decoded
            // to. A write needs the burst's next beat and room on both
            // of that peripheral's channels; a read needs room on its
            // one. A hole needs no room, and a write's beat to a hole
            // is taken and dropped.
            let cur = self.sel.get();
            let wbeat = self.busy & !self.rd & !self.sent;
            let rbeat = self.busy & self.rd & !self.sent;
            let w_off = w.peek().is_some();
            let wh = w.head();
            let mut w_room = Bit::One;
            let mut ar_room = Bit::One;
            for i in 0..N {
                let me = cur.bit(i);
                w_room = mux(me, aws[i].ready() & ws[i].ready(), w_room);
                ar_room = mux(me, ars[i].ready(), ar_room);
            }
            let w_go = wbeat & w_off & w_room;
            let _ = w.recv_if(wbeat & w_room);
            let ar_go = rbeat & ar_room;
            for i in 0..N {
                if (w_go & cur.bit(i)).to_bool() {
                    aws[i].send(LiteAw {
                        addr: self.addr.get(),
                        prot: self.prot.get(),
                    });
                    ws[i].send(LiteW {
                        data: wh.data,
                        strb: wh.strb,
                    });
                }
                if (ar_go & cur.bit(i)).to_bool() {
                    ars[i].send(LiteAr {
                        addr: self.addr.get(),
                        prot: self.prot.get(),
                    });
                }
            }
            // end{requests}
            // begin{answers}
            // The answer to the beat, from that peripheral, or the
            // bridge's own `DecErr` for a hole, there at once.
            let mut b_off = Bit::One;
            let mut b_resp = Resp::DecErr;
            let mut r_off = Bit::One;
            let mut r_data = U::<D>::from(0u8);
            let mut r_resp = Resp::DecErr;
            for i in 0..N {
                let me = cur.bit(i);
                b_off = mux(me, Bit::from(bs[i].peek().is_some()), b_off);
                b_resp = mux(me, bs[i].head().resp, b_resp);
                r_off = mux(me, Bit::from(rs[i].peek().is_some()), r_off);
                r_data = mux(me, rs[i].head().data, r_data);
                r_resp = mux(me, rs[i].head().resp, r_resp);
            }
            let last = self.left == 0;
            // A write's answer is taken when the burst's own response
            // has room, or when more beats are to come.
            let wwait = self.busy & !self.rd & self.sent;
            let b_can = wwait & (b.ready() | !last);
            let b_done = b_can & b_off;
            // A read's answer goes straight up as the burst's beat.
            let rwait = self.busy & self.rd & self.sent;
            let r_can = rwait & r.ready();
            let r_done = r_can & r_off;
            for i in 0..N {
                let _ = bs[i].recv_if(b_can & cur.bit(i));
                let _ = rs[i].recv_if(r_can & cur.bit(i));
            }
            // The burst's response keeps its first error.
            let clean = self.wresp.get() == Resp::Okay;
            let merged = mux(clean, b_resp, self.wresp.get());
            if (b_done & last).to_bool() {
                b.send(B {
                    id: self.xid.get(),
                    resp: merged,
                });
            }
            if r_done.to_bool() {
                r.send(R {
                    id: self.xid.get(),
                    data: r_data,
                    resp: r_resp,
                    last: Bit::from(last),
                });
            }
            // end{answers}
            // begin{drives}
            let done = b_done | r_done;
            with!(self <= {
                (take_ar | take_aw) ? {
                    busy: Bit::One,
                    rd: take_ar,
                    rfirst: !take_ar,
                    xid: mux(take_ar, arh.id, awh.id),
                    addr: mux(take_ar, arh.addr, awh.addr),
                    stride: new_stride,
                    left: mux(take_ar, arh.len, awh.len),
                    prot: mux(take_ar, arh.prot, awh.prot),
                    sel: new_sel,
                    sent: Bit::Zero,
                    wresp: Resp::Okay,
                },
                (w_go | ar_go) ? { sent: Bit::One },
                done ? {
                    sent: Bit::Zero,
                    addr: self.addr.get() + self.stride.get(),
                    left: self.left.get() - 1,
                    wresp: merged,
                },
                (done & last) ? { busy: Bit::Zero },
            });
            // end{drives}
        }
    }
}
// end{part}

/// The bridge against a model: bursts of one to four beats, reads and
/// writes, incrementing and fixed, to two peripherals and to a hole.
/// Every read reads what the writes left, a burst to the hole is
/// answered `DecErr` in as many beats as it asked for, and every beat
/// of a burst reaches its peripheral as a transaction of its own.
#[cfg(test)]
mod tests {
    use super::*;
    use crate::bus::axi::{axi, AxiHost, BurstKind, Link, Rd, Wr};
    use std::cell::RefCell;
    use std::collections::HashMap;
    use std::rc::Rc;
    use txhdl::comp::{join2, Clock, Running, Unit};

    type Host = AxiHost<16, 32, 4, 2, 4>;
    /// Two peripherals, a nibble each; everything else is a hole.
    struct TwoMap;
    impl AddrMap<2> for TwoMap {
        const RANGES: [(usize, usize); 2] =
            [(0x1000, 0xf000), (0x2000, 0xf000)];
    }
    type Bridge = LiteBridge<2, TwoMap, 16, 32, 4, 2>;

    /// A memory behind an AXI-Lite link, written as a simulation: it
    /// counts the transactions it serves, so the test can see that a
    /// burst arrived a beat at a time.
    async fn memory(
        per: LitePer<16, 32, 4>,
        words: Rc<RefCell<HashMap<u128, u128>>>,
        served: Rc<RefCell<usize>>,
    ) {
        let (aw, ar, w, b, r) = per;
        loop {
            DefaultClock::rising().await;
            if aw.peek().is_some() && w.peek().is_some() && b.ready().to_bool()
            {
                let a = aw.recv().unwrap();
                let d = w.recv().unwrap();
                words.borrow_mut().insert(a.addr.raw(), d.data.raw());
                b.send(LiteB { resp: Resp::Okay });
                *served.borrow_mut() += 1;
            }
            if ar.peek().is_some() && r.ready().to_bool() {
                let a = ar.recv().unwrap();
                let data = *words.borrow().get(&a.addr.raw()).unwrap_or(&0);
                r.send(LiteR {
                    data: U::from(data),
                    resp: Resp::Okay,
                });
                *served.borrow_mut() += 1;
            }
        }
    }

    #[test]
    fn bursts_cross_the_bridge_a_beat_at_a_time() {
        let Link {
            host,
            host_in,
            host_out,
            per_in,
            per_out,
            ..
        } = axi::<16, 32, 4, 2, 4>();
        let l0 = axi_lite::<16, 32, 4>();
        let l1 = axi_lite::<16, 32, 4>();
        let (aw0, ar0, w0, b0, r0) = l0.host;
        let (aw1, ar1, w1, b1, r1) = l1.host;
        let words = Rc::new(RefCell::new(HashMap::new()));
        let served0 = Rc::new(RefCell::new(0usize));
        let served1 = Rc::new(RefCell::new(0usize));
        let out = Rc::new(RefCell::new(Vec::<String>::new()));
        let o = out.clone();
        // Bursts of whole words, four bytes a beat, so the address
        // moves by four.
        let words_at = |a: u32, n: usize| {
            let mut rd = Rd::at(a, n);
            rd.size = U::from(2u8);
            rd
        };
        let write_at = |a: u32| {
            let mut wr = Wr::at(a);
            wr.size = U::from(2u8);
            wr
        };
        let client = async move {
            // Three words written at 0x1004 in one incrementing burst,
            // then read back in one burst of four from 0x1000.
            let vs = [U::from(0x11u32), U::from(0x22u32), U::from(0x33u32)];
            let wr = host.write(write_at(0x1004), &vs).await;
            assert_eq!(wr.done().await.resp, Resp::Okay);
            let rd = host.read(words_at(0x1000, 4)).await;
            let got = rd.done().await;
            assert_eq!(got.resp, Resp::Okay);
            let raw: Vec<u128> = got.data.iter().map(|d| d.raw()).collect();
            assert_eq!(raw, vec![0, 0x11, 0x22, 0x33]);
            // A fixed burst reads one word three times.
            let mut fixed = words_at(0x1008, 3);
            fixed.burst = BurstKind::Fixed;
            let got = host.read(fixed).await.done().await;
            let raw: Vec<u128> = got.data.iter().map(|d| d.raw()).collect();
            assert_eq!(raw, vec![0x22, 0x22, 0x22]);
            // The second peripheral, one word.
            let wr = host.write(Wr::at(0x2000u32), &[U::from(7u32)]).await;
            assert_eq!(wr.done().await.resp, Resp::Okay);
            let got = host.read(Rd::at(0x2000u32, 1)).await.done().await;
            assert_eq!(got.data[0].raw(), 7);
            // A hole: a write of two beats and a read of two, answered
            // by the bridge.
            let two = [U::from(1u32), U::from(2u32)];
            let wr = host.write(Wr::at(0x5000u32), &two).await;
            assert_eq!(wr.done().await.resp, Resp::DecErr);
            let got = host.read(Rd::at(0x5000u32, 2)).await.done().await;
            assert_eq!(got.resp, Resp::DecErr);
            assert_eq!(got.data.len(), 2, "a read of a hole, in beats");
            o.borrow_mut().push("done".to_string());
        };
        let mut h = Host::default();
        let mut bridge = Bridge::default();
        let (s0, s1) = (served0.clone(), served1.clone());
        let mut sim = Running::new(join2(
            join2(
                h.run(host_in, host_out),
                bridge.run(
                    (per_in.0, per_in.1, per_in.2, [b0, b1], [r0, r1]),
                    ([aw0, aw1], [ar0, ar1], [w0, w1], per_out.2, per_out.3),
                ),
            ),
            join2(
                client,
                join2(
                    memory(l0.per, words.clone(), s0),
                    memory(l1.per, words.clone(), s1),
                ),
            ),
        ));
        for _ in 0..400 {
            sim.cycle();
        }
        assert_eq!(*out.borrow(), vec!["done".to_string()], "the run ended");
        // Three beats written, four read, three read again: ten
        // transactions on the first peripheral, and two on the second.
        assert_eq!(*served0.borrow(), 10, "one transaction per beat");
        assert_eq!(*served1.borrow(), 2, "the second peripheral's");
    }
}
