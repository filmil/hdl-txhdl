// SPDX-License-Identifier: Apache-2.0
//! What BAR1 reaches: four words of registers behind the endpoint's
//! AXI master, and the unit of units that joins them to its pins.
use txhdl::comp::{chan, join2, Clock, DefaultClock, In, Out, Reg, Unit};
use txhdl::types::{Bit, U};
use txhdl::{lower, select, with, Trace};
use txhdl_parts::bus::axi::{
    Answer, Ar, Aw, AxiPer, PerPort, PerReq, Resp, B, R, W,
};
use txhdl_parts::bus::axi_pins::{AxiHostPins, AxiPins, AxiPinsIn, AxiPinsOut};

/// The identifier word, at offset 0: `TxHDL` in ASCII, and a version.
pub const IDENT: u64 = 0x5478_4844_4c00_0001;

// begin{regs}
/// Four 64-bit words behind a peripheral tracker, at offsets `0x0`,
/// `0x8`, `0x10` and `0x18` of BAR1: the identifier, read only; a
/// scratch word a host reads back; two LEDs, in the low bits; and a
/// count of the writes served, read only. It takes one request at a
/// time, as the timer does: a read is answered in the cycle it is
/// taken, and a write when its beat comes. The tracker sends it only
/// its own range, so it looks at bits 4 and 3 of an address alone.
#[derive(Trace, Default)]
pub struct BarRegs<const I: usize> {
    /// The scratch word.
    pub scratch: Reg<U<64>>,
    /// The LEDs' word, two bits of it.
    pub lights: Reg<U<2>>,
    /// Writes served, of any word.
    pub writes: Reg<U<32>>,
    /// A write taken and waiting for its beat: which word, and the
    /// identifier its answer carries.
    pub pend: Reg<U<1>>,
    /// The word the held write names.
    pub psel: Reg<U<2>>,
    /// The identifier of the held write.
    pub pid: Reg<U<I>>,
}

#[lower]
impl<const I: usize> Unit for BarRegs<I> {
    async fn run(
        &mut self,
        bus: PerPort<32, 64, 8, I>,
        (rst, leds): (In<Bit>, Out<U<2>>),
    ) {
        loop {
            DefaultClock::rising().await;
            let rst = rst.get();
            let q = bus.req.head();
            let qoff = bus.req.peek().is_some();
            let held = self.pend.get() == 1;
            let take_read = qoff & q.read & bus.r.ready() & !held;
            let take_write = qoff & !q.read & !held;
            let _ = bus.req.recv_if(take_read | take_write);
            let wh = bus.w.head();
            let wgo = held & bus.w.peek().is_some() & bus.ans.ready();
            let _ = bus.w.recv_if(wgo);
            let sel = q.addr.slice::<3, 2>();
            let word = select!(sel.raw() => {
                0 => U::<64>::from(IDENT),
                1 => self.scratch.get(),
                2 => self.lights.get().zext::<64>(),
                _ => self.writes.get().zext::<64>(),
            });
            let wsel = self.psel.get();
            let wdata = wh.data;
            with!(self <= {
                take_write ? {
                    pend: U::<1>::from(1u8),
                    psel: sel,
                    pid: q.id,
                },
                wgo ? {
                    pend: U::<1>::from(0u8),
                    writes: self.writes + 1,
                },
                wgo & (wsel == 1) ? scratch: wdata,
                wgo & (wsel == 2) ? lights: wdata.slice::<0, 2>(),
                rst ? {
                    pend: U::<1>::from(0u8),
                    scratch: U::<64>::from(0u8),
                    lights: U::<2>::from(0u8),
                    writes: U::<32>::from(0u8),
                },
            });
            if take_read.to_bool() {
                bus.r.send(R {
                    id: q.id,
                    data: word,
                    resp: Resp::Okay,
                    last: Bit::One,
                });
            }
            if wgo.to_bool() {
                bus.ans.send(Answer {
                    id: self.pid.get(),
                    resp: Resp::Okay,
                });
            }
            leds.set(self.lights);
        }
    }
}
// end{regs}

// begin{bar}
/// The endpoint's master pins, as the design's inputs, with its reset.
/// The widths are the endpoint's, on this side of it: 32-bit
/// addresses, 64-bit words, eight lanes, four-bit identifiers.
pub struct PcieBarIn {
    /// The design's reset, high while the endpoint's is low.
    pub rst: In<Bit>,
    /// `AWID`.
    pub awid: In<U<4>>,
    /// `AWADDR`, its low 32 bits.
    pub awaddr: In<U<32>>,
    /// `AWLEN`.
    pub awlen: In<U<8>>,
    /// `AWSIZE`.
    pub awsize: In<U<3>>,
    /// `AWBURST`.
    pub awburst: In<U<2>>,
    /// `AWLOCK`.
    pub awlock: In<Bit>,
    /// `AWCACHE`.
    pub awcache: In<U<4>>,
    /// `AWPROT`.
    pub awprot: In<U<3>>,
    /// `AWVALID`.
    pub awvalid: In<Bit>,
    /// `WDATA`.
    pub wdata: In<U<64>>,
    /// `WSTRB`.
    pub wstrb: In<U<8>>,
    /// `WLAST`.
    pub wlast: In<Bit>,
    /// `WVALID`.
    pub wvalid: In<Bit>,
    /// `BREADY`.
    pub bready: In<Bit>,
    /// `ARID`.
    pub arid: In<U<4>>,
    /// `ARADDR`, its low 32 bits.
    pub araddr: In<U<32>>,
    /// `ARLEN`.
    pub arlen: In<U<8>>,
    /// `ARSIZE`.
    pub arsize: In<U<3>>,
    /// `ARBURST`.
    pub arburst: In<U<2>>,
    /// `ARLOCK`.
    pub arlock: In<Bit>,
    /// `ARCACHE`.
    pub arcache: In<U<4>>,
    /// `ARPROT`.
    pub arprot: In<U<3>>,
    /// `ARVALID`.
    pub arvalid: In<Bit>,
    /// `RREADY`.
    pub rready: In<Bit>,
}

/// What the design drives: the answers on the endpoint's pins, and the
/// two LEDs.
pub struct PcieBarOut {
    /// `AWREADY`.
    pub awready: Out<Bit>,
    /// `WREADY`.
    pub wready: Out<Bit>,
    /// `BID`.
    pub bid: Out<U<4>>,
    /// `BRESP`.
    pub bresp: Out<U<2>>,
    /// `BVALID`.
    pub bvalid: Out<Bit>,
    /// `ARREADY`.
    pub arready: Out<Bit>,
    /// `RID`.
    pub rid: Out<U<4>>,
    /// `RDATA`.
    pub rdata: Out<U<64>>,
    /// `RRESP`.
    pub rresp: Out<U<2>>,
    /// `RLAST`.
    pub rlast: Out<Bit>,
    /// `RVALID`.
    pub rvalid: Out<Bit>,
    /// The LEDs' word.
    pub leds: Out<U<2>>,
}

/// Everything behind BAR1, as one lowered unit: the pins, a peripheral
/// tracker, and the registers. The board's top wires the endpoint's
/// master pins to its ports and its clock to the endpoint's user clock.
#[derive(Trace, Default)]
pub struct PcieBar {
    /// The pins, joined to the link's channels.
    pub pins: AxiPins<32, 64, 8, 4>,
    /// The peripheral tracker.
    pub per: AxiPer<32, 64, 8, 4>,
    /// The registers.
    pub regs: BarRegs<4>,
}

#[lower]
impl Unit for PcieBar {
    async fn run(
        &mut self,
        PcieBarIn {
            rst,
            awid,
            awaddr,
            awlen,
            awsize,
            awburst,
            awlock,
            awcache,
            awprot,
            awvalid,
            wdata,
            wstrb,
            wlast,
            wvalid,
            bready,
            arid,
            araddr,
            arlen,
            arsize,
            arburst,
            arlock,
            arcache,
            arprot,
            arvalid,
            rready,
        }: PcieBarIn,
        PcieBarOut {
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
            leds,
        }: PcieBarOut,
    ) {
        let (aw_tx, aw_rx) = chan::<Aw<32, 4>, DefaultClock>();
        let (ar_tx, ar_rx) = chan::<Ar<32, 4>, DefaultClock>();
        let (w_tx, w_rx) = chan::<W<64, 8>, DefaultClock>();
        let (b_tx, b_rx) = chan::<B<4>, DefaultClock>();
        let (r_tx, r_rx) = chan::<R<64, 4>, DefaultClock>();
        let (req_tx, req_rx) = chan::<PerReq<32, 4>, DefaultClock>();
        let (wd_tx, wd_rx) = chan::<W<64, 8>, DefaultClock>();
        let (ans_tx, ans_rx) = chan::<Answer<4>, DefaultClock>();
        let (rb_tx, rb_rx) = chan::<R<64, 4>, DefaultClock>();
        join2(
            join2(
                self.pins.run(
                    AxiPinsIn {
                        pins: AxiHostPins {
                            awid,
                            awaddr,
                            awlen,
                            awsize,
                            awburst,
                            awlock,
                            awcache,
                            awprot,
                            awvalid,
                            wdata,
                            wstrb,
                            wlast,
                            wvalid,
                            bready,
                            arid,
                            araddr,
                            arlen,
                            arsize,
                            arburst,
                            arlock,
                            arcache,
                            arprot,
                            arvalid,
                            rready,
                        },
                        b: b_rx,
                        r: r_rx,
                    },
                    AxiPinsOut {
                        aw: aw_tx,
                        ar: ar_tx,
                        w: w_tx,
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
                ),
                self.per.run(
                    (aw_rx, ar_rx, w_rx, ans_rx, rb_rx),
                    (req_tx, wd_tx, b_tx, r_tx),
                ),
            ),
            self.regs.run(
                PerPort {
                    req: req_rx,
                    w: wd_rx,
                    ans: ans_tx,
                    r: rb_tx,
                },
                (rst, leds),
            ),
        )
        .await;
    }
}
// end{bar}
