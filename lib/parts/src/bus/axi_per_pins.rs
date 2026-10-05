// SPDX-License-Identifier: Apache-2.0
//! The link's peripheral end, joined to an AXI4 peripheral's pins.
//!
//! The mirror of [`AxiPins`]: a core from elsewhere that is an AXI4
//! peripheral, such as AMD's DDR3 controller generated with its AXI4
//! port, has pins, a `valid` and a `ready` per channel and a wire per
//! field, and the parts here speak the link as channels. [`AxiPerPins`]
//! stands where [`AxiPer`] would, on the link's peripheral end, and holds
//! nothing: an address phase or a write beat at the head of its channel
//! is the peripheral's `valid` and fields, taken when the peripheral's
//! `ready` is high; a response or a read beat the peripheral offers is
//! sent on its channel when the channel has room, and that room is the
//! peripheral's `ready`.
//!
//! Every field of the link's address phase goes out on a pin but one:
//! `AxREGION`. AMD's controller has no region pins, as many peripherals
//! do not, since the region is how an interconnect tells one decoded
//! range of a peripheral from another and this part joins one link to
//! one peripheral. So the link's region is not carried, and a
//! peripheral that wants one gets it as a constant where it is
//! instantiated. `AxQOS` is carried, since the controller has it.
//! The widths are stated, as everywhere in this library: `A` the
//! address width, `D` the data width, `S` the strobe width, which is
//! `D / 8`, and `I` the identifier width.
//!
//! [`AxiPins`]: super::axi_pins::AxiPins
//! [`AxiPer`]: super::axi::AxiPer
use crate::bus::axi::{Ar, Aw, BurstKind, Resp, B, R, W};
use txhdl::comp::{Clock, DefaultClock, In, Out, Rx, Tx, Unit};
use txhdl::types::{Bit, U};
use txhdl::{lower, select, Ports, Trace};

// begin{ports}
/// What the peripheral drives: its eleven pins, named as AXI4 names
/// them. A struct of its own, as [`AxiHostPins`] is, so that a design
/// holding a peripheral's pins among its ports passes them on whole.
///
/// [`AxiHostPins`]: super::axi_pins::AxiHostPins
#[derive(Ports)]
pub struct AxiPerDriven<const D: usize, const I: usize> {
    /// The peripheral takes a write address phase.
    pub awready: In<Bit>,
    /// The peripheral takes a write beat.
    pub wready: In<Bit>,
    /// The burst a write response answers.
    pub bid: In<U<I>>,
    /// How the write went, as AXI4 encodes it.
    pub bresp: In<U<2>>,
    /// A write response offered.
    pub bvalid: In<Bit>,
    /// The peripheral takes a read address phase.
    pub arready: In<Bit>,
    /// The burst a read beat belongs to.
    pub rid: In<U<I>>,
    /// The word read.
    pub rdata: In<U<D>>,
    /// How the beat went, as AXI4 encodes it.
    pub rresp: In<U<2>>,
    /// The last beat of the burst.
    pub rlast: In<Bit>,
    /// A read beat offered.
    pub rvalid: In<Bit>,
}

/// What the peripheral drives, and the link's address phases and write
/// beats: the unit's inputs.
pub struct AxiPerPinsIn<
    const A: usize,
    const D: usize,
    const S: usize,
    const I: usize,
> {
    /// The peripheral's pins.
    pub pins: AxiPerDriven<D, I>,
    /// The write address phases, from the link.
    pub aw: Rx<Aw<A, I>>,
    /// The read address phases, from the link.
    pub ar: Rx<Ar<A, I>>,
    /// The write beats, from the link.
    pub w: Rx<W<D, S>>,
}

/// The answers to the link, and what the peripheral is driven with:
/// the unit's outputs.
pub struct AxiPerPinsOut<
    const A: usize,
    const D: usize,
    const S: usize,
    const I: usize,
> {
    /// The write responses, to the link.
    pub b: Tx<B<I>>,
    /// The read beats, to the link.
    pub r: Tx<R<D, I>>,
    /// `AWID`.
    pub awid: Out<U<I>>,
    /// `AWADDR`.
    pub awaddr: Out<U<A>>,
    /// `AWLEN`: beats in the burst, less one.
    pub awlen: Out<U<8>>,
    /// `AWSIZE`: bytes per beat, as a power of two.
    pub awsize: Out<U<3>>,
    /// `AWBURST`, as AXI4 encodes it.
    pub awburst: Out<U<2>>,
    /// `AWLOCK`.
    pub awlock: Out<Bit>,
    /// `AWCACHE`.
    pub awcache: Out<U<4>>,
    /// `AWPROT`.
    pub awprot: Out<U<3>>,
    /// `AWQOS`.
    pub awqos: Out<U<4>>,
    /// A write address phase offered.
    pub awvalid: Out<Bit>,
    /// `WDATA`.
    pub wdata: Out<U<D>>,
    /// `WSTRB`.
    pub wstrb: Out<U<S>>,
    /// `WLAST`.
    pub wlast: Out<Bit>,
    /// A write beat offered.
    pub wvalid: Out<Bit>,
    /// The write response channel has room.
    pub bready: Out<Bit>,
    /// `ARID`.
    pub arid: Out<U<I>>,
    /// `ARADDR`.
    pub araddr: Out<U<A>>,
    /// `ARLEN`.
    pub arlen: Out<U<8>>,
    /// `ARSIZE`.
    pub arsize: Out<U<3>>,
    /// `ARBURST`.
    pub arburst: Out<U<2>>,
    /// `ARLOCK`.
    pub arlock: Out<Bit>,
    /// `ARCACHE`.
    pub arcache: Out<U<4>>,
    /// `ARPROT`.
    pub arprot: Out<U<3>>,
    /// `ARQOS`.
    pub arqos: Out<U<4>>,
    /// A read address phase offered.
    pub arvalid: Out<Bit>,
    /// The read beat channel has room.
    pub rready: Out<Bit>,
}
// end{ports}

/// The link's peripheral end joined to a peripheral's AXI4 pins; see
/// the module.
#[derive(Trace, Default)]
pub struct AxiPerPins<
    const A: usize,
    const D: usize,
    const S: usize,
    const I: usize,
> {}

// begin{unit}
#[lower]
impl<const A: usize, const D: usize, const S: usize, const I: usize> Unit
    for AxiPerPins<A, D, S, I>
{
    async fn run(
        &mut self,
        inp: AxiPerPinsIn<A, D, S, I>,
        outp: AxiPerPinsOut<A, D, S, I>,
    ) {
        loop {
            DefaultClock::rising().await;
            // An address phase or a beat at the head of its channel is
            // the peripheral's `valid` and fields, taken when the
            // peripheral is ready.
            let awh = inp.aw.head();
            let arh = inp.ar.head();
            let wh = inp.w.head();
            let awburst = select!(awh.burst => {
                BurstKind::Fixed => U::<2>::from(0u8),
                BurstKind::Incr => U::<2>::from(1u8),
                BurstKind::Wrap => U::<2>::from(2u8),
                _ => U::<2>::from(3u8),
            });
            let arburst = select!(arh.burst => {
                BurstKind::Fixed => U::<2>::from(0u8),
                BurstKind::Incr => U::<2>::from(1u8),
                BurstKind::Wrap => U::<2>::from(2u8),
                _ => U::<2>::from(3u8),
            });
            outp.awvalid.set(Bit::from(inp.aw.peek().is_some()));
            outp.awid.set(awh.id);
            outp.awaddr.set(awh.addr);
            outp.awlen.set(awh.len);
            outp.awsize.set(awh.size);
            outp.awburst.set(awburst);
            outp.awlock.set(awh.lock);
            outp.awcache.set(awh.cache);
            outp.awprot.set(awh.prot);
            outp.awqos.set(awh.qos);
            outp.arvalid.set(Bit::from(inp.ar.peek().is_some()));
            outp.arid.set(arh.id);
            outp.araddr.set(arh.addr);
            outp.arlen.set(arh.len);
            outp.arsize.set(arh.size);
            outp.arburst.set(arburst);
            outp.arlock.set(arh.lock);
            outp.arcache.set(arh.cache);
            outp.arprot.set(arh.prot);
            outp.arqos.set(arh.qos);
            outp.wvalid.set(Bit::from(inp.w.peek().is_some()));
            outp.wdata.set(wh.data);
            outp.wstrb.set(wh.strb);
            outp.wlast.set(wh.last);
            let _ = inp.aw.recv_if(inp.pins.awready.get());
            let _ = inp.ar.recv_if(inp.pins.arready.get());
            let _ = inp.w.recv_if(inp.pins.wready.get());
            // A response or a read beat the peripheral offers goes onto
            // its channel when the channel has room; the room is the
            // peripheral's `ready`.
            let b_room = outp.b.ready();
            let r_room = outp.r.ready();
            let bresp_pin = inp.pins.bresp.get();
            let bresp = select!(bresp_pin.raw() => {
                0 => Resp::Okay,
                1 => Resp::ExOkay,
                2 => Resp::SlvErr,
                _ => Resp::DecErr,
            });
            let rresp_pin = inp.pins.rresp.get();
            let rresp = select!(rresp_pin.raw() => {
                0 => Resp::Okay,
                1 => Resp::ExOkay,
                2 => Resp::SlvErr,
                _ => Resp::DecErr,
            });
            if (inp.pins.bvalid.get() & b_room).to_bool() {
                outp.b.send(B {
                    id: inp.pins.bid.get(),
                    resp: bresp,
                });
            }
            if (inp.pins.rvalid.get() & r_room).to_bool() {
                outp.r.send(R {
                    id: inp.pins.rid.get(),
                    data: inp.pins.rdata.get(),
                    resp: rresp,
                    last: inp.pins.rlast.get(),
                });
            }
            outp.bready.set(b_room);
            outp.rready.set(r_room);
        }
    }
}
// end{unit}

/// A peripheral on AXI4 pins, written as a simulation: a memory behind
/// the pins, the way a core from elsewhere is one. A test or an example
/// drives [`AxiPerPins`] with it, and runs it ahead of the unit in the
/// join, as [`PinHost`] is run, since the pins are wires the unit reads
/// in the same step. So it records an address phase or a beat a step
/// later, from what the unit drove while it was ready, and it offers its
/// answers one at a time, a response or a beat leaving when the unit's
/// `ready` was high in the step it was offered.
///
/// It is ready for everything at once and answers at once, unless it is
/// given a timing: then it is not ready until a warm-up has passed, as a
/// controller is not until it has calibrated, and it answers a read and
/// a write a number of steps after taking them, as a controller does.
/// That is what lets it stand for one in simulation.
///
/// [`PinHost`]: super::axi_pins::sim::PinHost
pub mod sim {
    use super::{AxiPerDriven, AxiPerPinsIn, AxiPerPinsOut};
    use crate::bus::axi::{Ar, Aw, B, R, W};
    use std::cell::RefCell;
    use std::collections::{HashMap, VecDeque};
    use std::rc::Rc;
    use txhdl::comp::{signal, Clock, DefaultClock, In, Out, Rx, Tx};
    use txhdl::types::{Bit, U};

    /// The pins a memory reads: what the unit drives, less the fields it
    /// does not use. Every burst is taken as incrementing words, so
    /// `AxSIZE` and `AxBURST` are not among them, and nor are `AxLOCK`,
    /// `AxCACHE`, `AxPROT` and `AxQOS`.
    pub struct PinRamIn<
        const A: usize,
        const D: usize,
        const S: usize,
        const I: usize,
    > {
        /// `AWID`.
        pub awid: In<U<I>>,
        /// `AWADDR`.
        pub awaddr: In<U<A>>,
        /// `AWLEN`.
        pub awlen: In<U<8>>,
        /// `AWVALID`.
        pub awvalid: In<Bit>,
        /// `WDATA`.
        pub wdata: In<U<D>>,
        /// `WSTRB`.
        pub wstrb: In<U<S>>,
        /// `WLAST`.
        pub wlast: In<Bit>,
        /// `WVALID`.
        pub wvalid: In<Bit>,
        /// `BREADY`.
        pub bready: In<Bit>,
        /// `ARID`.
        pub arid: In<U<I>>,
        /// `ARADDR`.
        pub araddr: In<U<A>>,
        /// `ARLEN`.
        pub arlen: In<U<8>>,
        /// `ARVALID`.
        pub arvalid: In<Bit>,
        /// `RREADY`.
        pub rready: In<Bit>,
    }

    /// The pins a memory drives: its readies and its answers.
    pub struct PinRamOut<const D: usize, const I: usize> {
        /// `AWREADY`.
        pub awready: Out<Bit>,
        /// `WREADY`.
        pub wready: Out<Bit>,
        /// `ARREADY`.
        pub arready: Out<Bit>,
        /// `BID`.
        pub bid: Out<U<I>>,
        /// `BRESP`.
        pub bresp: Out<U<2>>,
        /// `BVALID`.
        pub bvalid: Out<Bit>,
        /// `RID`.
        pub rid: Out<U<I>>,
        /// `RDATA`.
        pub rdata: Out<U<D>>,
        /// `RRESP`.
        pub rresp: Out<U<2>>,
        /// `RLAST`.
        pub rlast: Out<Bit>,
        /// `RVALID`.
        pub rvalid: Out<Bit>,
    }

    /// The peripheral's ends of the pins, and its memory.
    pub struct PinRam<
        const A: usize,
        const D: usize,
        const S: usize,
        const I: usize,
    > {
        inp: PinRamIn<A, D, S, I>,
        out: PinRamOut<D, I>,
        mem: Rc<RefCell<HashMap<u128, U<D>>>>,
        words: u128,
        wraps: bool,
        read_latency: u64,
        write_latency: u64,
        warmup: u64,
    }

    impl<const A: usize, const D: usize, const S: usize, const I: usize>
        PinRam<A, D, S, I>
    {
        /// A memory of `words` words on the given pins, every word zero,
        /// ready at once and answering at once.
        pub fn on(
            inp: PinRamIn<A, D, S, I>,
            out: PinRamOut<D, I>,
            words: u128,
        ) -> Self {
            PinRam {
                inp,
                out,
                mem: Rc::new(RefCell::new(HashMap::new())),
                words,
                wraps: false,
                read_latency: 0,
                write_latency: 0,
                warmup: 0,
            }
        }

        /// The same memory, not ready for its first `warmup` steps, with
        /// a read's first beat offered `read` steps after its address was
        /// taken and a write's response `write` steps after its last
        /// beat was.
        pub fn timed(mut self, read: u64, write: u64, warmup: u64) -> Self {
            self.read_latency = read;
            self.write_latency = write;
            self.warmup = warmup;
            self
        }

        /// The same memory, with an address past its end taken modulo
        /// its size rather than refused, as a controller whose address is
        /// narrower than the link's takes it.
        pub fn wrapping(mut self) -> Self {
            self.wraps = true;
            self
        }

        /// The memory's `i`th word.
        pub fn word(&self, i: u128) -> U<D> {
            self.mem.borrow().get(&i).copied().unwrap_or_default()
        }

        /// A handle on the memory, for a test to read after the run: the
        /// words written, by index.
        pub fn memory(&self) -> Rc<RefCell<HashMap<u128, U<D>>>> {
            self.mem.clone()
        }

        /// The index of a word, and whether the memory has it.
        fn index(&self, at: u128) -> (u128, bool) {
            if self.wraps {
                (at % self.words, true)
            } else {
                (at, at < self.words)
            }
        }

        /// Answer bursts for ever: a write's beats go into the memory and
        /// its response follows the last; a read's beats come out one a
        /// step. A word past the memory's end is not written, and is
        /// answered `SLVERR`, as is a read of one, unless the memory
        /// wraps.
        pub async fn serve(self) {
            let shift = S.trailing_zeros();
            let inp = &self.inp;
            let out = &self.out;
            // Write bursts awaiting their beats: identifier, first word,
            // beats so far, and whether any was past the end.
            let mut writes: VecDeque<(U<I>, u128, u128, bool)> =
                VecDeque::new();
            let mut beats: VecDeque<(U<D>, U<S>, Bit)> = VecDeque::new();
            // Answers, each with the step it may be offered from.
            let mut bq: VecDeque<(U<I>, u8, u64)> = VecDeque::new();
            let mut rq: VecDeque<(U<I>, u128, bool, u64)> = VecDeque::new();
            let (mut b_offered, mut r_offered) = (false, false);
            let mut ready = false;
            let mut now: u64 = 0;
            loop {
                DefaultClock::rising().await;
                now += 1;
                // What the unit drove in the last step, taken if this
                // memory was ready then, and whether it took what was
                // offered then.
                if ready && inp.awvalid.get().to_bool() {
                    let first = inp.awaddr.get().raw() >> shift;
                    writes.push_back((inp.awid.get(), first, 0, false));
                }
                if ready && inp.wvalid.get().to_bool() {
                    beats.push_back((
                        inp.wdata.get(),
                        inp.wstrb.get(),
                        inp.wlast.get(),
                    ));
                }
                if ready && inp.arvalid.get().to_bool() {
                    let first = inp.araddr.get().raw() >> shift;
                    let n = inp.arlen.get().raw() + 1;
                    let due = now + self.read_latency;
                    for k in 0..n {
                        rq.push_back((
                            inp.arid.get(),
                            first + k,
                            k + 1 == n,
                            due,
                        ));
                    }
                }
                if b_offered && inp.bready.get().to_bool() {
                    bq.pop_front();
                }
                if r_offered && inp.rready.get().to_bool() {
                    rq.pop_front();
                }
                while !writes.is_empty() && !beats.is_empty() {
                    let (data, strb, last) = beats.pop_front().unwrap();
                    let (id, first, k, mut bad) = writes.pop_front().unwrap();
                    let (at, ok) = self.index(first + k);
                    if ok {
                        let mut mem = self.mem.borrow_mut();
                        let mut word =
                            mem.get(&at).copied().unwrap_or_default().raw();
                        for lane in 0..S {
                            if (strb.raw() >> lane) & 1 == 1 {
                                let m = 0xffu128 << (8 * lane);
                                word = (word & !m) | (data.raw() & m);
                            }
                        }
                        mem.insert(at, U::<D>::new(word));
                    } else {
                        bad = true;
                    }
                    if last.to_bool() {
                        let due = now + self.write_latency;
                        bq.push_back((id, if bad { 2 } else { 0 }, due));
                    } else {
                        writes.push_front((id, first, k + 1, bad));
                    }
                }
                // Ready once warm, and the answers at the head on offer
                // once they are due.
                ready = now > self.warmup;
                out.awready.set(Bit::from_bool(ready));
                out.wready.set(Bit::from_bool(ready));
                out.arready.set(Bit::from_bool(ready));
                b_offered =
                    matches!(bq.front(), Some(&(_, _, due)) if due <= now);
                if let Some(&(id, resp, _)) = bq.front() {
                    out.bid.set(id);
                    out.bresp.set(U::<2>::from(resp));
                }
                out.bvalid.set(Bit::from_bool(b_offered));
                r_offered =
                    matches!(rq.front(), Some(&(_, _, _, due)) if due <= now);
                if let Some(&(id, at, last, _)) = rq.front() {
                    let (at, ok) = self.index(at);
                    let data = if ok { self.word(at) } else { U::<D>::new(0) };
                    out.rid.set(id);
                    out.rdata.set(data);
                    out.rresp.set(U::<2>::from(if ok { 0u8 } else { 2u8 }));
                    out.rlast.set(Bit::from_bool(last));
                }
                out.rvalid.set(Bit::from_bool(r_offered));
            }
        }
    }

    /// Make the pins between an [`AxiPerPins`](super::AxiPerPins) and a
    /// [`PinRam`] of `words` words, with the link's channel ends given to
    /// the unit where they belong. The pins the memory leaves unread are
    /// wires nobody reads.
    #[allow(clippy::type_complexity)]
    pub fn pins<
        const A: usize,
        const D: usize,
        const S: usize,
        const I: usize,
    >(
        aw: Rx<Aw<A, I>>,
        ar: Rx<Ar<A, I>>,
        w: Rx<W<D, S>>,
        b: Tx<B<I>>,
        r: Tx<R<D, I>>,
        words: usize,
    ) -> (
        PinRam<A, D, S, I>,
        AxiPerPinsIn<A, D, S, I>,
        AxiPerPinsOut<A, D, S, I>,
    ) {
        fn wire<T: txhdl::types::Value + Copy + Default>() -> (Out<T>, In<T>) {
            signal::<T, DefaultClock>()
        }
        let (awid_o, awid) = wire::<U<I>>();
        let (awaddr_o, awaddr) = wire::<U<A>>();
        let (awlen_o, awlen) = wire::<U<8>>();
        let (awvalid_o, awvalid) = wire::<Bit>();
        let (wdata_o, wdata) = wire::<U<D>>();
        let (wstrb_o, wstrb) = wire::<U<S>>();
        let (wlast_o, wlast) = wire::<Bit>();
        let (wvalid_o, wvalid) = wire::<Bit>();
        let (bready_o, bready) = wire::<Bit>();
        let (arid_o, arid) = wire::<U<I>>();
        let (araddr_o, araddr) = wire::<U<A>>();
        let (arlen_o, arlen) = wire::<U<8>>();
        let (arvalid_o, arvalid) = wire::<Bit>();
        let (rready_o, rready) = wire::<Bit>();
        let (awready_o, awready) = wire::<Bit>();
        let (wready_o, wready) = wire::<Bit>();
        let (arready_o, arready) = wire::<Bit>();
        let (bid_o, bid) = wire::<U<I>>();
        let (bresp_o, bresp) = wire::<U<2>>();
        let (bvalid_o, bvalid) = wire::<Bit>();
        let (rid_o, rid) = wire::<U<I>>();
        let (rdata_o, rdata) = wire::<U<D>>();
        let (rresp_o, rresp) = wire::<U<2>>();
        let (rlast_o, rlast) = wire::<Bit>();
        let (rvalid_o, rvalid) = wire::<Bit>();
        (
            PinRam::on(
                PinRamIn {
                    awid,
                    awaddr,
                    awlen,
                    awvalid,
                    wdata,
                    wstrb,
                    wlast,
                    wvalid,
                    bready,
                    arid,
                    araddr,
                    arlen,
                    arvalid,
                    rready,
                },
                PinRamOut {
                    awready: awready_o,
                    wready: wready_o,
                    arready: arready_o,
                    bid: bid_o,
                    bresp: bresp_o,
                    bvalid: bvalid_o,
                    rid: rid_o,
                    rdata: rdata_o,
                    rresp: rresp_o,
                    rlast: rlast_o,
                    rvalid: rvalid_o,
                },
                words as u128,
            ),
            AxiPerPinsIn {
                pins: AxiPerDriven {
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
                aw,
                ar,
                w,
            },
            AxiPerPinsOut {
                b,
                r,
                awid: awid_o,
                awaddr: awaddr_o,
                awlen: awlen_o,
                awsize: wire::<U<3>>().0,
                awburst: wire::<U<2>>().0,
                awlock: wire::<Bit>().0,
                awcache: wire::<U<4>>().0,
                awprot: wire::<U<3>>().0,
                awqos: wire::<U<4>>().0,
                awvalid: awvalid_o,
                wdata: wdata_o,
                wstrb: wstrb_o,
                wlast: wlast_o,
                wvalid: wvalid_o,
                bready: bready_o,
                arid: arid_o,
                araddr: araddr_o,
                arlen: arlen_o,
                arsize: wire::<U<3>>().0,
                arburst: wire::<U<2>>().0,
                arlock: wire::<Bit>().0,
                arcache: wire::<U<4>>().0,
                arprot: wire::<U<3>>().0,
                arqos: wire::<U<4>>().0,
                arvalid: arvalid_o,
                rready: rready_o,
            },
        )
    }
}

#[cfg(test)]
mod tests {
    use super::sim::pins;
    use super::*;
    use crate::bus::axi::{axi, AxiHost, Link, Rd, Wr};
    use std::cell::RefCell;
    use std::rc::Rc;
    use txhdl::comp::{join2, Running};

    /// A host on the link writes a burst into a memory on AXI4 pins,
    /// reads it back across the burst's ends, and reads past the
    /// memory's end, which the peripheral refuses.
    #[test]
    fn a_host_on_the_link_reaches_a_peripheral_on_pins() {
        let Link {
            host,
            host_in,
            host_out,
            per_in,
            per_out,
            ..
        } = axi::<16, 32, 4, 2, 4>();
        let (aw, ar, w, _, _) = per_in;
        let (_, _, b, r) = per_out;
        let (ram, inp, outp) = pins::<16, 32, 4, 2>(aw, ar, w, b, r, 16);
        let mem = ram.memory();
        let done = Rc::new(RefCell::new(false));
        let d = done.clone();
        let client = async move {
            let words = [U::from(0x11u32), U::from(0x22u32), U::from(0x33u32)];
            let got = host.write(Wr::at(0x8u32), &words).await.done().await;
            assert_eq!(got.resp, Resp::Okay);
            let got = host.read(Rd::at(0x4u32, 4)).await.done().await;
            assert_eq!(got.resp, Resp::Okay);
            let raw: Vec<u128> = got.data.iter().map(|x| x.raw()).collect();
            assert_eq!(raw, vec![0, 0x11, 0x22, 0x33]);
            // Past the memory's sixteen words: the peripheral refuses.
            let got = host.read(Rd::at(0x40u32, 1)).await.done().await;
            assert_eq!(got.resp, Resp::SlvErr);
            *d.borrow_mut() = true;
        };
        let mut tracker = AxiHost::<16, 32, 4, 2, 4>::default();
        let mut pinned = AxiPerPins::<16, 32, 4, 2>::default();
        // The memory first, then the unit: the unit reads the pins the
        // memory drives in the same step.
        let mut sim = Running::new(join2(
            client,
            join2(
                tracker.run(host_in, host_out),
                join2(ram.serve(), pinned.run(inp, outp)),
            ),
        ));
        for _ in 0..300 {
            sim.cycle();
            if *done.borrow() {
                break;
            }
        }
        assert!(*done.borrow(), "every burst was answered");
        assert_eq!(mem.borrow()[&3].raw(), 0x22, "the second word landed");
    }

    /// The cycle a write of `beats` words is answered in, through a
    /// memory with the timing given, and the cycles the read after it
    /// takes.
    fn timed(read: u64, write: u64, warmup: u64, beats: usize) -> (u64, u64) {
        let Link {
            host,
            host_in,
            host_out,
            per_in,
            per_out,
            ..
        } = axi::<16, 32, 4, 2, 4>();
        let (aw, ar, w, _, _) = per_in;
        let (_, _, b, r) = per_out;
        let (ram, inp, outp) = pins::<16, 32, 4, 2>(aw, ar, w, b, r, 64);
        let ram = ram.timed(read, write, warmup);
        let at = Rc::new(RefCell::new((0u64, 0u64)));
        let a = at.clone();
        // The clock runs on across runs in one thread, so count from here.
        let t0 = txhdl::comp::now();
        let client = async move {
            let words: Vec<U<32>> =
                (0..beats).map(|i| U::from(i as u32)).collect();
            let got = host.write(Wr::at(0x0u32), &words).await.done().await;
            assert_eq!(got.resp, Resp::Okay);
            a.borrow_mut().0 = (txhdl::comp::now() - t0) / DefaultClock::PERIOD;
            let got = host.read(Rd::at(0x0u32, beats)).await.done().await;
            assert_eq!(got.data, words);
            a.borrow_mut().1 = (txhdl::comp::now() - t0) / DefaultClock::PERIOD;
        };
        let mut tracker = AxiHost::<16, 32, 4, 2, 4>::default();
        let mut pinned = AxiPerPins::<16, 32, 4, 2>::default();
        let mut sim = Running::new(join2(
            client,
            join2(
                tracker.run(host_in, host_out),
                join2(ram.serve(), pinned.run(inp, outp)),
            ),
        ));
        for _ in 0..1000 {
            sim.cycle();
        }
        let (w, r) = *at.borrow();
        assert!(w > 0 && r > w, "both answered: {w} {r}");
        (w, r - w)
    }

    /// A warm-up holds everything off until it has passed, and a latency
    /// delays an answer by exactly itself: a read's first beat by the
    /// read latency, after which the burst streams a beat a cycle, and a
    /// write's response by the write latency.
    #[test]
    fn a_timed_memory_waits_and_then_answers_late() {
        let (w0, r0) = timed(0, 0, 0, 8);
        let (w1, r1) = timed(0, 0, 40, 8);
        assert!(w1 >= 40, "the write waited for the warm-up: {w1}");
        assert_eq!(r1, r0, "the read after it did not");
        let (w2, r2) = timed(10, 3, 0, 8);
        assert_eq!(r2, r0 + 10, "a read is ten cycles later");
        assert_eq!(w2, w0 + 3, "a write is three cycles later");
        let (_, r3) = timed(10, 3, 0, 16);
        assert_eq!(r3 - r2, 8, "eight more beats, a cycle each");
    }
}
