// SPDX-License-Identifier: Apache-2.0
//! A host's AXI4 pins, joined to the link's channels.
//!
//! A core from elsewhere that is an AXI4 host, such as a PCIe
//! endpoint whose BAR is an AXI master, has pins: a `valid` and a
//! `ready` per channel and a wire per field, named as the AXI4
//! specification names them. The parts here speak the link as
//! channels, each beat one transaction. [`AxiPins`] stands between
//! the two. It holds nothing: an address phase or a write beat the
//! host offers is sent on the channel when the channel has room, and
//! that room is the pin's `ready`; a response or a read beat at the
//! head of its channel is the pin's `valid` and fields, taken when the
//! host's `ready` is high. So it goes where [`AxiHost`] would, with
//! [`AxiPer`] or a router behind it.
//!
//! The host drives no `AxQOS` and no `AxREGION`, as an endpoint's
//! master does not, so both go out as zero. The widths are stated, as
//! everywhere in this library: `A` the address width, `D` the data
//! width, `S` the strobe width, which is `D / 8`, and `I` the
//! identifier width.
//!
//! [`AxiHost`]: super::axi::AxiHost
//! [`AxiPer`]: super::axi::AxiPer
use crate::bus::axi::{Addr, Ar, Aw, BurstKind, Resp, B, R, W};
use txhdl::comp::{Clock, DefaultClock, In, Out, Rx, Tx, Unit};
use txhdl::types::{Bit, U};
use txhdl::{lower, select, Trace};

// begin{ports}
/// What the host drives, and the answers coming back to it from the
/// link: the unit's inputs.
pub struct AxiPinsIn<
    const A: usize,
    const D: usize,
    const S: usize,
    const I: usize,
> {
    /// The write address phase's pins.
    pub awid: In<U<I>>,
    /// The address a write burst starts at.
    pub awaddr: In<U<A>>,
    /// Beats in the write burst, less one.
    pub awlen: In<U<8>>,
    /// Bytes per beat, as a power of two.
    pub awsize: In<U<3>>,
    /// The burst type, as AXI4 encodes it.
    pub awburst: In<U<2>>,
    /// An exclusive access.
    pub awlock: In<Bit>,
    /// `AWCACHE`.
    pub awcache: In<U<4>>,
    /// `AWPROT`.
    pub awprot: In<U<3>>,
    /// A write address phase offered.
    pub awvalid: In<Bit>,
    /// The word written.
    pub wdata: In<U<D>>,
    /// Which of its bytes are meant.
    pub wstrb: In<U<S>>,
    /// The last beat of the burst.
    pub wlast: In<Bit>,
    /// A write beat offered.
    pub wvalid: In<Bit>,
    /// The host takes a write response.
    pub bready: In<Bit>,
    /// The read address phase's pins, as the write's.
    pub arid: In<U<I>>,
    /// The address a read burst starts at.
    pub araddr: In<U<A>>,
    /// Beats in the read burst, less one.
    pub arlen: In<U<8>>,
    /// Bytes per beat, as a power of two.
    pub arsize: In<U<3>>,
    /// The burst type.
    pub arburst: In<U<2>>,
    /// An exclusive access.
    pub arlock: In<Bit>,
    /// `ARCACHE`.
    pub arcache: In<U<4>>,
    /// `ARPROT`.
    pub arprot: In<U<3>>,
    /// A read address phase offered.
    pub arvalid: In<Bit>,
    /// The host takes a read beat.
    pub rready: In<Bit>,
    /// The write responses from the link.
    pub b: Rx<B<I>>,
    /// The read beats from the link.
    pub r: Rx<R<D, I>>,
}

/// What goes to the link, and the answers to the host: the unit's
/// outputs.
pub struct AxiPinsOut<
    const A: usize,
    const D: usize,
    const S: usize,
    const I: usize,
> {
    /// The write address phases, to the link.
    pub aw: Tx<Aw<A, I>>,
    /// The read address phases, to the link.
    pub ar: Tx<Ar<A, I>>,
    /// The write beats, to the link.
    pub w: Tx<W<D, S>>,
    /// The write address channel has room.
    pub awready: Out<Bit>,
    /// The write data channel has room.
    pub wready: Out<Bit>,
    /// The burst a write response answers.
    pub bid: Out<U<I>>,
    /// How the write went, as AXI4 encodes it.
    pub bresp: Out<U<2>>,
    /// A write response offered.
    pub bvalid: Out<Bit>,
    /// The read address channel has room.
    pub arready: Out<Bit>,
    /// The burst a read beat belongs to.
    pub rid: Out<U<I>>,
    /// The word read.
    pub rdata: Out<U<D>>,
    /// How the beat went, as AXI4 encodes it.
    pub rresp: Out<U<2>>,
    /// The last beat of the burst.
    pub rlast: Out<Bit>,
    /// A read beat offered.
    pub rvalid: Out<Bit>,
}
// end{ports}

/// A host's AXI4 pins joined to the link's channels; see the module.
#[derive(Trace, Default)]
pub struct AxiPins<
    const A: usize,
    const D: usize,
    const S: usize,
    const I: usize,
> {}

// begin{unit}
#[lower]
impl<const A: usize, const D: usize, const S: usize, const I: usize> Unit
    for AxiPins<A, D, S, I>
{
    async fn run(
        &mut self,
        inp: AxiPinsIn<A, D, S, I>,
        outp: AxiPinsOut<A, D, S, I>,
    ) {
        loop {
            DefaultClock::rising().await;
            // An address phase or a beat goes onto its channel in the
            // step the host offers it and the channel has room; the
            // room is the host's `ready`.
            let aw_room = outp.aw.ready();
            let ar_room = outp.ar.ready();
            let w_room = outp.w.ready();
            let awburst = inp.awburst.get();
            let arburst = inp.arburst.get();
            let awkind = select!(awburst.raw() => {
                0 => BurstKind::Fixed,
                1 => BurstKind::Incr,
                2 => BurstKind::Wrap,
                _ => BurstKind::Reserved,
            });
            let arkind = select!(arburst.raw() => {
                0 => BurstKind::Fixed,
                1 => BurstKind::Incr,
                2 => BurstKind::Wrap,
                _ => BurstKind::Reserved,
            });
            if (inp.awvalid.get() & aw_room).to_bool() {
                outp.aw.send(Addr {
                    id: inp.awid.get(),
                    addr: inp.awaddr.get(),
                    len: inp.awlen.get(),
                    size: inp.awsize.get(),
                    burst: awkind,
                    lock: inp.awlock.get(),
                    cache: inp.awcache.get(),
                    prot: inp.awprot.get(),
                    qos: U::<4>::from(0u8),
                    region: U::<4>::from(0u8),
                });
            }
            if (inp.arvalid.get() & ar_room).to_bool() {
                outp.ar.send(Addr {
                    id: inp.arid.get(),
                    addr: inp.araddr.get(),
                    len: inp.arlen.get(),
                    size: inp.arsize.get(),
                    burst: arkind,
                    lock: inp.arlock.get(),
                    cache: inp.arcache.get(),
                    prot: inp.arprot.get(),
                    qos: U::<4>::from(0u8),
                    region: U::<4>::from(0u8),
                });
            }
            if (inp.wvalid.get() & w_room).to_bool() {
                outp.w.send(W {
                    data: inp.wdata.get(),
                    strb: inp.wstrb.get(),
                    last: inp.wlast.get(),
                });
            }
            outp.awready.set(aw_room);
            outp.arready.set(ar_room);
            outp.wready.set(w_room);
            // A response or a read beat at the head of its channel is
            // the host's `valid`, taken when the host is ready.
            let bh = inp.b.head();
            let rh = inp.r.head();
            let bresp = select!(bh.resp => {
                Resp::Okay => U::<2>::from(0u8),
                Resp::ExOkay => U::<2>::from(1u8),
                Resp::SlvErr => U::<2>::from(2u8),
                _ => U::<2>::from(3u8),
            });
            let rresp = select!(rh.resp => {
                Resp::Okay => U::<2>::from(0u8),
                Resp::ExOkay => U::<2>::from(1u8),
                Resp::SlvErr => U::<2>::from(2u8),
                _ => U::<2>::from(3u8),
            });
            outp.bvalid.set(Bit::from(inp.b.peek().is_some()));
            outp.bid.set(bh.id);
            outp.bresp.set(bresp);
            outp.rvalid.set(Bit::from(inp.r.peek().is_some()));
            outp.rid.set(rh.id);
            outp.rdata.set(rh.data);
            outp.rresp.set(rresp);
            outp.rlast.set(rh.last);
            let _ = inp.b.recv_if(inp.bready.get());
            let _ = inp.r.recv_if(inp.rready.get());
        }
    }
}
// end{unit}

/// A host on AXI4 pins, written as a simulation: what a core from
/// elsewhere does at its pins, one burst at a time. A test or an
/// example drives [`AxiPins`] with it, and runs it ahead of the unit
/// in the join, since the pins are wires the unit reads in the same
/// step.
pub mod sim {
    use super::{AxiPinsIn, AxiPinsOut};
    use crate::bus::axi::{Ar, Aw, Resp, B, R, W};
    use txhdl::comp::{signal, Clock, DefaultClock, In, Out, Rx, Tx};
    use txhdl::types::{Bit, U};

    /// The host's ends of the pins: what it drives and what it reads.
    pub struct PinHost<
        const A: usize,
        const D: usize,
        const S: usize,
        const I: usize,
    > {
        aw: (Out<U<I>>, Out<U<A>>, Out<U<8>>, Out<U<3>>, Out<U<2>>),
        awvalid: Out<Bit>,
        w: (Out<U<D>>, Out<U<S>>, Out<Bit>),
        wvalid: Out<Bit>,
        bready: Out<Bit>,
        ar: (Out<U<I>>, Out<U<A>>, Out<U<8>>, Out<U<3>>, Out<U<2>>),
        arvalid: Out<Bit>,
        rready: Out<Bit>,
        awready: In<Bit>,
        wready: In<Bit>,
        bid: In<U<I>>,
        bresp: In<U<2>>,
        bvalid: In<Bit>,
        arready: In<Bit>,
        rid: In<U<I>>,
        rdata: In<U<D>>,
        rresp: In<U<2>>,
        rlast: In<Bit>,
        rvalid: In<Bit>,
    }

    /// What a burst came back with: its identifier, its response, and
    /// a read's words.
    pub struct Answer<const D: usize, const I: usize> {
        /// The identifier the answer carried.
        pub id: U<I>,
        /// The burst's response: a write's one, or a read's first that
        /// is not `Okay`.
        pub resp: Resp,
        /// A read's words, in order; none for a write.
        pub data: Vec<U<D>>,
    }

    fn resp(v: U<2>) -> Resp {
        match v.raw() {
            0 => Resp::Okay,
            1 => Resp::ExOkay,
            2 => Resp::SlvErr,
            _ => Resp::DecErr,
        }
    }

    impl<const A: usize, const D: usize, const S: usize, const I: usize>
        PinHost<A, D, S, I>
    {
        /// Write `data` from `addr`, an incrementing burst of whole
        /// words under identifier `id`, and wait for its response. The
        /// address phase goes first and then the beats, as a simple
        /// host sends them.
        pub async fn write(
            &self,
            id: u32,
            addr: u128,
            data: &[U<D>],
        ) -> Answer<D, I> {
            let size = (S.trailing_zeros()) as u8;
            self.aw.0.set(U::<I>::from(id));
            self.aw.1.set(U::<A>::new(addr));
            self.aw.2.set(U::<8>::from(data.len() as u32 - 1));
            self.aw.3.set(U::<3>::from(size));
            self.aw.4.set(U::<2>::from(1u8));
            self.awvalid.set(Bit::One);
            // `ready` is read a step late: high then, the address
            // phase was taken in that step.
            loop {
                DefaultClock::rising().await;
                if self.awready.get().to_bool() {
                    break;
                }
            }
            self.awvalid.set(Bit::Zero);
            for (i, d) in data.iter().enumerate() {
                self.w.0.set(*d);
                self.w.1.set(U::<S>::new((1u128 << S) - 1));
                self.w.2.set(Bit::from_bool(i + 1 == data.len()));
                self.wvalid.set(Bit::One);
                loop {
                    DefaultClock::rising().await;
                    if self.wready.get().to_bool() {
                        break;
                    }
                }
            }
            self.wvalid.set(Bit::Zero);
            self.bready.set(Bit::One);
            loop {
                DefaultClock::rising().await;
                if self.bvalid.get().to_bool() {
                    break;
                }
            }
            self.bready.set(Bit::Zero);
            Answer {
                id: self.bid.get(),
                resp: resp(self.bresp.get()),
                data: Vec::new(),
            }
        }

        /// Read `n` whole words from `addr`, an incrementing burst under
        /// identifier `id`, and wait for its last beat.
        pub async fn read(
            &self,
            id: u32,
            addr: u128,
            n: usize,
        ) -> Answer<D, I> {
            let size = (S.trailing_zeros()) as u8;
            self.ar.0.set(U::<I>::from(id));
            self.ar.1.set(U::<A>::new(addr));
            self.ar.2.set(U::<8>::from(n as u32 - 1));
            self.ar.3.set(U::<3>::from(size));
            self.ar.4.set(U::<2>::from(1u8));
            self.arvalid.set(Bit::One);
            loop {
                DefaultClock::rising().await;
                if self.arready.get().to_bool() {
                    break;
                }
            }
            self.arvalid.set(Bit::Zero);
            self.rready.set(Bit::One);
            let mut data = Vec::new();
            let mut worst = Resp::Okay;
            let mut id = U::<I>::from(0u8);
            loop {
                DefaultClock::rising().await;
                if self.rvalid.get().to_bool() {
                    data.push(self.rdata.get());
                    id = self.rid.get();
                    let r = resp(self.rresp.get());
                    if worst == Resp::Okay {
                        worst = r;
                    }
                    if self.rlast.get().to_bool() {
                        break;
                    }
                }
            }
            self.rready.set(Bit::Zero);
            Answer {
                id,
                resp: worst,
                data,
            }
        }
    }

    /// Make the pins: the host's ends, and the unit's two sides, with
    /// the link's channel ends given to the unit where they belong.
    #[allow(clippy::type_complexity)]
    pub fn pins<
        const A: usize,
        const D: usize,
        const S: usize,
        const I: usize,
    >(
        aw: Tx<Aw<A, I>>,
        ar: Tx<Ar<A, I>>,
        w: Tx<W<D, S>>,
        b: Rx<B<I>>,
        r: Rx<R<D, I>>,
    ) -> (
        PinHost<A, D, S, I>,
        AxiPinsIn<A, D, S, I>,
        AxiPinsOut<A, D, S, I>,
    ) {
        fn wire<T: txhdl::types::Value + Copy + Default>() -> (Out<T>, In<T>) {
            signal::<T, DefaultClock>()
        }
        let (awid, awid_i) = wire::<U<I>>();
        let (awaddr, awaddr_i) = wire::<U<A>>();
        let (awlen, awlen_i) = wire::<U<8>>();
        let (awsize, awsize_i) = wire::<U<3>>();
        let (awburst, awburst_i) = wire::<U<2>>();
        let (awvalid, awvalid_i) = wire::<Bit>();
        let (wdata, wdata_i) = wire::<U<D>>();
        let (wstrb, wstrb_i) = wire::<U<S>>();
        let (wlast, wlast_i) = wire::<Bit>();
        let (wvalid, wvalid_i) = wire::<Bit>();
        let (bready, bready_i) = wire::<Bit>();
        let (arid, arid_i) = wire::<U<I>>();
        let (araddr, araddr_i) = wire::<U<A>>();
        let (arlen, arlen_i) = wire::<U<8>>();
        let (arsize, arsize_i) = wire::<U<3>>();
        let (arburst, arburst_i) = wire::<U<2>>();
        let (arvalid, arvalid_i) = wire::<Bit>();
        let (rready, rready_i) = wire::<Bit>();
        let (awready_o, awready) = wire::<Bit>();
        let (wready_o, wready) = wire::<Bit>();
        let (bid_o, bid) = wire::<U<I>>();
        let (bresp_o, bresp) = wire::<U<2>>();
        let (bvalid_o, bvalid) = wire::<Bit>();
        let (arready_o, arready) = wire::<Bit>();
        let (rid_o, rid) = wire::<U<I>>();
        let (rdata_o, rdata) = wire::<U<D>>();
        let (rresp_o, rresp) = wire::<U<2>>();
        let (rlast_o, rlast) = wire::<Bit>();
        let (rvalid_o, rvalid) = wire::<Bit>();
        // The pins this host leaves at rest: no lock, no cache hints,
        // and unprivileged, secure, data access.
        let rest = || wire::<Bit>().1;
        let rest4 = || wire::<U<4>>().1;
        let rest3 = || wire::<U<3>>().1;
        (
            PinHost {
                aw: (awid, awaddr, awlen, awsize, awburst),
                awvalid,
                w: (wdata, wstrb, wlast),
                wvalid,
                bready,
                ar: (arid, araddr, arlen, arsize, arburst),
                arvalid,
                rready,
                awready,
                wready,
                bid,
                bresp,
                bvalid,
                arready,
                rid,
                rdata,
                rresp,
                rlast,
                rvalid,
            },
            AxiPinsIn {
                awid: awid_i,
                awaddr: awaddr_i,
                awlen: awlen_i,
                awsize: awsize_i,
                awburst: awburst_i,
                awlock: rest(),
                awcache: rest4(),
                awprot: rest3(),
                awvalid: awvalid_i,
                wdata: wdata_i,
                wstrb: wstrb_i,
                wlast: wlast_i,
                wvalid: wvalid_i,
                bready: bready_i,
                arid: arid_i,
                araddr: araddr_i,
                arlen: arlen_i,
                arsize: arsize_i,
                arburst: arburst_i,
                arlock: rest(),
                arcache: rest4(),
                arprot: rest3(),
                arvalid: arvalid_i,
                rready: rready_i,
                b,
                r,
            },
            AxiPinsOut {
                aw,
                ar,
                w,
                awready: awready_o,
                wready: wready_o,
                bid: bid_o,
                bresp: bresp_o,
                bvalid: bvalid_o,
                arready: arready_o,
                rid: rid_o,
                rdata: rdata_o,
                rresp: rresp_o,
                rlast: rlast_o,
                rvalid: rvalid_o,
            },
        )
    }
}

#[cfg(test)]
mod tests {
    use super::sim::pins;
    use super::*;
    use crate::bus::axi::sim::Ram;
    use crate::bus::axi::{axi, AxiPer, Link};
    use std::cell::RefCell;
    use std::rc::Rc;
    use txhdl::comp::{join2, Running};

    /// A host on pins writes a burst into a RAM behind a peripheral
    /// tracker, reads it back, and reads past the RAM's end.
    #[test]
    fn a_host_on_pins_reaches_a_peripheral_through_the_link() {
        let Link {
            host_in,
            host_out,
            per,
            per_in,
            per_out,
            ..
        } = axi::<16, 32, 4, 2, 4>();
        let (aw, ar, w, _, _, _) = host_out;
        let (_, _, b, r, _) = host_in;
        let (host, inp, outp) = pins::<16, 32, 4, 2>(aw, ar, w, b, r);
        let ram = Ram::<16, 32, 4, 2>::new(16);
        let seen = ram.clone();
        let done = Rc::new(RefCell::new(false));
        let d = done.clone();
        let client = async move {
            let words = [U::from(0x11u32), U::from(0x22u32), U::from(0x33u32)];
            let got = host.write(1, 0x8, &words).await;
            assert_eq!(got.resp, Resp::Okay);
            assert_eq!(got.id.raw(), 1);
            let got = host.read(2, 0x4, 4).await;
            assert_eq!(got.resp, Resp::Okay);
            assert_eq!(got.id.raw(), 2);
            let raw: Vec<u128> = got.data.iter().map(|x| x.raw()).collect();
            assert_eq!(raw, vec![0, 0x11, 0x22, 0x33]);
            // Past the RAM's sixteen words: the peripheral refuses.
            let got = host.read(3, 0x40, 1).await;
            assert_eq!(got.resp, Resp::SlvErr);
            *d.borrow_mut() = true;
        };
        let mut pinned = AxiPins::<16, 32, 4, 2>::default();
        let mut tracker = AxiPer::<16, 32, 4, 2>::default();
        let mut sim = Running::new(join2(
            client,
            join2(
                pinned.run(inp, outp),
                join2(tracker.run(per_in, per_out), ram.serve(per, 4)),
            ),
        ));
        for _ in 0..300 {
            sim.cycle();
            if *done.borrow() {
                break;
            }
        }
        assert!(*done.borrow(), "the host's bursts were all answered");
        assert_eq!(seen.word(3).raw(), 0x22, "the second word landed");
    }
}
