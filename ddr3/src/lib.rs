// SPDX-License-Identifier: Apache-2.0
//! The board's DDR3 memory as an AXI peripheral, with the controller
//! a module from elsewhere.
//!
//! [`Ddr3`] is AMD's MIG 7 Series controller generated with its AXI4
//! port, which the build generates (`//ddr3:ddr3_mig_axi`), behind the
//! thin wrapper in `hdl/ddr3_axi32.v`, which ties off what a design has
//! no reason to drive. It is a foreign unit: its netlist is an instance
//! of that module, and its `run` is a model of it, a memory on the same
//! AXI4 pins that is not ready while it calibrates and answers after the
//! controller's latency. The model does not drive the memory's pins;
//! nothing in a simulation reads them, and the pins only have to be in
//! the netlist. The controller makes the design's clock from the
//! board's, and the model leaves that clock and its reset alone.
//!
//! [`Ddr3Per`] is the peripheral a design puts on its link: the link's
//! peripheral end put onto the controller's pins by `AxiPerPins`, with
//! no tracker in front, since the controller keeps its own. It is a unit
//! of units and lowers as one, with the controller's module instantiated
//! in it and the memory's pins among its ports, so a board top has only
//! to join those pins to the package's, give it the board's clock, and
//! take the design's clock back from it.
pub mod bw;
pub mod sim;

use txhdl::comp::trace::Kind;
use txhdl::comp::{
    join2, mux, signal, Clock, DefaultClock, In, Out, Pad, Reg, Rx, Tx, Unit,
};
use txhdl::netlist::{foreign, Lower, Lowered};
use txhdl::types::{Bit, U};
use txhdl::{lower, select, with, Trace};
use txhdl_parts::bus::axi::{Ar, Aw, Resp, B, R, W};
use txhdl_parts::bus::axi_per_pins::sim::{PinRam, PinRamIn, PinRamOut};
use txhdl_parts::bus::axi_per_pins::{
    AxiPerDriven, AxiPerPins, AxiPerPinsIn, AxiPerPinsOut,
};

/// The memory's words as an address: a gigabyte of them is two to the
/// twenty-eighth.
pub const AW: usize = 28;

/// The controller's inputs: the board's 200 MHz clock, which the
/// controller runs the memory from and takes as its delay reference;
/// its reset, active high at power-on and never again; and the AXI4
/// pins a host drives, address phases, write beats and the two readies
/// of the answers, as `AxiPerPins` drives them. The design's clock is
/// the controller's own making, bound to the module's clock pin, and is
/// not a port here.
pub type CtlIn = (
    In<Bit>,
    In<Bit>,
    In<U<5>>,
    In<U<32>>,
    In<U<8>>,
    In<U<3>>,
    In<U<2>>,
    In<Bit>,
    In<U<4>>,
    In<U<3>>,
    In<U<4>>,
    In<Bit>,
    In<U<32>>,
    In<U<4>>,
    In<Bit>,
    In<Bit>,
    In<Bit>,
    In<U<5>>,
    In<U<32>>,
    In<U<8>>,
    In<U<3>>,
    In<U<2>>,
    In<Bit>,
    In<U<4>>,
    In<U<3>>,
    In<U<4>>,
    In<Bit>,
    In<Bit>,
    In<U<5>>,
    In<U<32>>,
    In<U<8>>,
    In<Bit>,
    In<Bit>,
);

/// The controller's outputs: the AXI4 pins it drives, its readies and
/// its answers; calibration done; the design's clock and its reset,
/// which the controller makes; the memory's clock pair, reset, clock
/// enable, chip select, the three command strobes, row address, bank,
/// strobe masks and termination; and the memory's data and strobe pads.
pub type CtlOut = (
    Out<Bit>,
    Out<Bit>,
    Out<U<5>>,
    Out<U<2>>,
    Out<Bit>,
    Out<Bit>,
    Out<U<5>>,
    Out<U<32>>,
    Out<U<2>>,
    Out<Bit>,
    Out<Bit>,
    Out<Bit>,
    Out<U<5>>,
    Out<U<256, 2>>,
    Out<U<2>>,
    Out<Bit>,
    Out<Bit>,
    Out<Bit>,
    Out<Bit>,
    Out<Bit>,
    Out<Bit>,
    Out<Bit>,
    Out<Bit>,
    Out<Bit>,
    Out<Bit>,
    Out<Bit>,
    Out<Bit>,
    Out<Bit>,
    Out<U<15>>,
    Out<U<3>>,
    Out<U<4>>,
    Out<Bit>,
    Pad<U<32>>,
    Pad<U<4>>,
    Pad<U<4>>,
);

// begin{ctl}
/// The controller: AMD's MIG 7 Series with its AXI4 port, behind the
/// wrapper, which calibrates in hardware and, in simulation, is the
/// variant with the fast calibration the build compiles into its
/// library, so it takes no parameter for either.
#[derive(Trace, Default)]
pub struct Ddr3 {
    /// Whether calibration is done, in the model.
    pub calibrated: Reg<Bit>,
    /// The model's memory, which a [`sim::WideRead`] may share.
    pub words: sim::Words,
}

impl Ddr3 {
    /// A handle on the model's memory: for a [`sim::WideRead`] to read
    /// beside it, or a test to read after the run.
    pub fn memory(&self) -> sim::Words {
        self.words.clone()
    }
}

impl Lower for Ddr3 {
    fn lowered_as(name: &str) -> Lowered {
        foreign(
            name,
            "ddr3_axi32",
            &[
                ("i_sys_clk", Kind::In, 1),
                ("i_sys_rst", Kind::In, 1),
                ("i_awid", Kind::In, 5),
                ("i_awaddr", Kind::In, 32),
                ("i_awlen", Kind::In, 8),
                ("i_awsize", Kind::In, 3),
                ("i_awburst", Kind::In, 2),
                ("i_awlock", Kind::In, 1),
                ("i_awcache", Kind::In, 4),
                ("i_awprot", Kind::In, 3),
                ("i_awqos", Kind::In, 4),
                ("i_awvalid", Kind::In, 1),
                ("i_wdata", Kind::In, 32),
                ("i_wstrb", Kind::In, 4),
                ("i_wlast", Kind::In, 1),
                ("i_wvalid", Kind::In, 1),
                ("i_bready", Kind::In, 1),
                ("i_arid", Kind::In, 5),
                ("i_araddr", Kind::In, 32),
                ("i_arlen", Kind::In, 8),
                ("i_arsize", Kind::In, 3),
                ("i_arburst", Kind::In, 2),
                ("i_arlock", Kind::In, 1),
                ("i_arcache", Kind::In, 4),
                ("i_arprot", Kind::In, 3),
                ("i_arqos", Kind::In, 4),
                ("i_arvalid", Kind::In, 1),
                ("i_rready", Kind::In, 1),
                ("i_xarid", Kind::In, 5),
                ("i_xaraddr", Kind::In, 32),
                ("i_xarlen", Kind::In, 8),
                ("i_xarvalid", Kind::In, 1),
                ("i_xrready", Kind::In, 1),
                ("o_awready", Kind::Out, 1),
                ("o_wready", Kind::Out, 1),
                ("o_bid", Kind::Out, 5),
                ("o_bresp", Kind::Out, 2),
                ("o_bvalid", Kind::Out, 1),
                ("o_arready", Kind::Out, 1),
                ("o_rid", Kind::Out, 5),
                ("o_rdata", Kind::Out, 32),
                ("o_rresp", Kind::Out, 2),
                ("o_rlast", Kind::Out, 1),
                ("o_rvalid", Kind::Out, 1),
                ("o_xarready", Kind::Out, 1),
                ("o_xrid", Kind::Out, 5),
                ("o_xrdata", Kind::Out, 256),
                ("o_xrresp", Kind::Out, 2),
                ("o_xrlast", Kind::Out, 1),
                ("o_xrvalid", Kind::Out, 1),
                ("o_calib_complete", Kind::Out, 1),
                ("o_ui_clk", Kind::Out, 1),
                ("o_ui_rst", Kind::Out, 1),
                ("o_ddr3_clk_p", Kind::Out, 1),
                ("o_ddr3_clk_n", Kind::Out, 1),
                ("o_ddr3_reset_n", Kind::Out, 1),
                ("o_ddr3_cke", Kind::Out, 1),
                ("o_ddr3_cs_n", Kind::Out, 1),
                ("o_ddr3_ras_n", Kind::Out, 1),
                ("o_ddr3_cas_n", Kind::Out, 1),
                ("o_ddr3_we_n", Kind::Out, 1),
                ("o_ddr3_addr", Kind::Out, 15),
                ("o_ddr3_ba_addr", Kind::Out, 3),
                ("o_ddr3_dm", Kind::Out, 4),
                ("o_ddr3_odt", Kind::Out, 1),
                ("io_ddr3_dq", Kind::Pad, 32),
                ("io_ddr3_dqs", Kind::Pad, 4),
                ("io_ddr3_dqs_n", Kind::Pad, 4),
            ],
            &[],
            &[("i_ui_clk", DefaultClock::NAME)],
        )
    }
}
// end{ctl}

/// Cycles the model is not ready for, before it has calibrated.
pub const MODEL_WARMUP: u32 = 64;

/// Cycles the model takes over a read, from the step it sees the address
/// phase to the step it offers the first beat. `//ddr3/sim:wrapper_test`
/// measured the controller against the Micron models: a read's first
/// beat came 25 cycles after its address phase was taken, a burst's 27,
/// and a read right behind a write 63. The model sees the address a
/// cycle after it is taken, so 24 is the common case.
pub const MODEL_READ_LATENCY: u32 = 24;

/// Cycles the model takes over a write, from the step it sees the last
/// beat to the step it offers the response: the controller answered 3
/// cycles after the last beat was taken, every time, in the same run.
pub const MODEL_WRITE_LATENCY: u32 = 2;

impl Unit<CtlIn, CtlOut> for Ddr3 {
    async fn run(
        &mut self,
        (
            _sys_clk,
            _sys_rst,
            awid,
            awaddr,
            awlen,
            _awsize,
            _awburst,
            _awlock,
            _awcache,
            _awprot,
            _awqos,
            awvalid,
            wdata,
            wstrb,
            wlast,
            wvalid,
            bready,
            arid,
            araddr,
            arlen,
            _arsize,
            _arburst,
            _arlock,
            _arcache,
            _arprot,
            _arqos,
            arvalid,
            rready,
            xarid,
            xaraddr,
            xarlen,
            xarvalid,
            xrready,
        ): CtlIn,
        (
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
            xarready,
            xrid,
            xrdata,
            xrresp,
            xrlast,
            xrvalid,
            calib,
            ..,
        ): CtlOut,
    ) {
        // The memory: the whole gigabyte, its address the link's taken
        // modulo the memory's size as the port's thirty bits take it,
        // not ready until calibrated, and answering after the
        // controller's latency. A read takes its words when its address
        // is taken, so a write that lands while it waits is not in them,
        // as the controller may answer (issue 1424).
        let mem = PinRam::<32, 32, 4, 5>::on(
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
                awready,
                wready,
                arready,
                bid,
                bresp,
                bvalid,
                rid,
                rdata,
                rresp,
                rlast,
                rvalid,
            },
            1 << AW,
        )
        .wrapping()
        .sharing(self.words.0.clone())
        .reading_at_address()
        .timed(
            MODEL_READ_LATENCY as u64,
            MODEL_WRITE_LATENCY as u64,
            MODEL_WARMUP as u64,
        );
        // The wide read port, on the same words (issue 1696).
        let wide = sim::serve_wide_pins(
            self.words.clone(),
            (xarid, xaraddr, xarlen, xarvalid, xrready),
            (xarready, xrid, xrdata, xrresp, xrlast, xrvalid),
            MODEL_WARMUP as u64,
        );
        let mut left = MODEL_WARMUP;
        let calibrated = &self.calibrated;
        join2(join2(mem.serve(), wide), async move {
            loop {
                DefaultClock::rising().await;
                calib.set(calibrated.get());
                calibrated.set(Bit::from_bool(left <= 1));
                left = left.saturating_sub(1);
            }
        })
        .await;
    }
}

// begin{wide}
/// The wide read port's channels on the controller's wide pins (issue
/// 1696): the address channel's head offered on the `AR` pins and taken
/// when `ARREADY` was high, and each beat the controller offers put on
/// the `R` channel when the channel has room, which is `RREADY`. It is
/// read only, so there is no write side.
///
/// Under the design's reset and after it, while beats are still owed, it
/// offers no new address and takes and drops the beats, as `AxiPerPins`
/// does, since the controller runs through a reset of the design: a read
/// asked before the reset is not answered after it.
#[derive(Trace, Default)]
pub struct WidePins {
    /// Beats asked for that have not come.
    pub owe: Reg<U<14>>,
    /// Draining after a reset.
    pub drain: Reg<Bit>,
}

#[lower]
impl Unit for WidePins {
    async fn run(
        &mut self,
        (rst, ar, arready, rid, rdata, rresp, rlast, rvalid): (
            In<Bit>,
            Rx<Ar<32, 5>>,
            In<Bit>,
            In<U<5>>,
            In<U<256, 2>>,
            In<U<2>>,
            In<Bit>,
            In<Bit>,
        ),
        (r, arid, araddr, arlen, arvalid, rready): (
            Tx<R<256, 5, 2>>,
            Out<U<5>>,
            Out<U<32>>,
            Out<U<8>>,
            Out<Bit>,
            Out<Bit>,
        ),
    ) {
        loop {
            DefaultClock::rising().await;
            let owe = self.owe.get();
            let drain = self.drain.get() | rst.get();
            let h = ar.head();
            let offer = Bit::from(ar.peek().is_some()) & !drain;
            arvalid.set(offer);
            arid.set(h.id);
            araddr.set(h.addr);
            arlen.set(h.len);
            let ar_go = offer & arready.get();
            let _ = ar.recv_if(ar_go);
            let room = r.ready();
            // Draining, every beat is taken and dropped.
            let take = drain | room;
            rready.set(take);
            let resp_pin = rresp.get();
            let resp = select!(resp_pin.raw() => {
                0 => Resp::Okay,
                1 => Resp::ExOkay,
                2 => Resp::SlvErr,
                _ => Resp::DecErr,
            });
            if (rvalid.get() & room & !drain).to_bool() {
                r.send(R {
                    id: rid.get(),
                    data: rdata.get(),
                    resp,
                    last: rlast.get(),
                });
            }
            // A beat counts only while one is owed, as in `AxiPerPins`, so
            // the pins before the controller is ready are masked.
            let zero = U::<14>::from(0u16);
            let r_go = rvalid.get() & take & Bit::from(owe != zero);
            let asked = h.len.zext::<14>() + U::<14>::from(1u16);
            let next = mux(ar_go, owe + asked, owe);
            let next = mux(r_go, next - U::<14>::from(1u16), next);
            with!(self <= {
                owe: next,
                drain: rst.get() | (self.drain.get() & Bit::from(next != zero)),
            });
        }
    }
}
// end{wide}

// begin{per}
/// The memory as a peripheral: the link's peripheral end on the
/// controller's pins.
#[derive(Trace, Default)]
pub struct Ddr3Per {
    pub pins: AxiPerPins<32, 32, 4, 5>,
    pub ctl: Ddr3,
    /// The wide read port's channels on the controller's wide pins.
    pub wide: WidePins,
}

/// The controller's side towards the board (issue 726): the reset, the
/// AXI4 link's five channels, and the wide read port's two. Taken apart
/// in `run`, so each port keeps its name.
pub struct Ddr3PerBus {
    pub rst: In<Bit>,
    pub aw: Rx<Aw<32, 5>>,
    pub ar: Rx<Ar<32, 5>>,
    pub w: Rx<W<32, 4>>,
    pub b: Tx<B<5>>,
    pub r: Tx<R<32, 5>>,
    pub xar: Rx<Ar<32, 5>>,
    pub xr: Tx<R<256, 5, 2>>,
}

/// The controller's side towards the memory (issue 726): its clock and
/// reset in, calibration and the user interface's clock and reset out,
/// the DDR3 command lines, the row and bank address, the data mask and
/// termination, and the data and strobe pads.
pub struct Ddr3PerPins {
    pub sys_clk: In<Bit>,
    pub sys_rst: In<Bit>,
    pub calib: Out<Bit>,
    pub ui_clk: Out<Bit>,
    pub ui_rst: Out<Bit>,
    pub ck_p: Out<Bit>,
    pub ck_n: Out<Bit>,
    pub mem_rst_n: Out<Bit>,
    pub cke: Out<Bit>,
    pub cs_n: Out<Bit>,
    pub ras_n: Out<Bit>,
    pub cas_n: Out<Bit>,
    pub we_n: Out<Bit>,
    pub row: Out<U<15>>,
    pub bank: Out<U<3>>,
    pub dm: Out<U<4>>,
    pub odt: Out<Bit>,
    pub dq: Pad<U<32>>,
    pub dqs: Pad<U<4>>,
    pub dqs_n: Pad<U<4>>,
}

#[lower]
impl Unit for Ddr3Per {
    async fn run(
        &mut self,
        Ddr3PerBus {
            rst,
            aw,
            ar,
            w,
            b,
            r,
            xar,
            xr,
        }: Ddr3PerBus,
        Ddr3PerPins {
            sys_clk,
            sys_rst,
            calib,
            ui_clk,
            ui_rst,
            ck_p,
            ck_n,
            mem_rst_n,
            cke,
            cs_n,
            ras_n,
            cas_n,
            we_n,
            row,
            bank,
            dm,
            odt,
            dq,
            dqs,
            dqs_n,
        }: Ddr3PerPins,
    ) {
        // What the unit drives and the controller reads.
        let (awid_o, awid_i) = signal::<U<5>, DefaultClock>();
        let (awaddr_o, awaddr_i) = signal::<U<32>, DefaultClock>();
        let (awlen_o, awlen_i) = signal::<U<8>, DefaultClock>();
        let (awsize_o, awsize_i) = signal::<U<3>, DefaultClock>();
        let (awburst_o, awburst_i) = signal::<U<2>, DefaultClock>();
        let (awlock_o, awlock_i) = signal::<Bit, DefaultClock>();
        let (awcache_o, awcache_i) = signal::<U<4>, DefaultClock>();
        let (awprot_o, awprot_i) = signal::<U<3>, DefaultClock>();
        let (awqos_o, awqos_i) = signal::<U<4>, DefaultClock>();
        let (awvalid_o, awvalid_i) = signal::<Bit, DefaultClock>();
        let (wdata_o, wdata_i) = signal::<U<32>, DefaultClock>();
        let (wstrb_o, wstrb_i) = signal::<U<4>, DefaultClock>();
        let (wlast_o, wlast_i) = signal::<Bit, DefaultClock>();
        let (wvalid_o, wvalid_i) = signal::<Bit, DefaultClock>();
        let (bready_o, bready_i) = signal::<Bit, DefaultClock>();
        let (arid_o, arid_i) = signal::<U<5>, DefaultClock>();
        let (araddr_o, araddr_i) = signal::<U<32>, DefaultClock>();
        let (arlen_o, arlen_i) = signal::<U<8>, DefaultClock>();
        let (arsize_o, arsize_i) = signal::<U<3>, DefaultClock>();
        let (arburst_o, arburst_i) = signal::<U<2>, DefaultClock>();
        let (arlock_o, arlock_i) = signal::<Bit, DefaultClock>();
        let (arcache_o, arcache_i) = signal::<U<4>, DefaultClock>();
        let (arprot_o, arprot_i) = signal::<U<3>, DefaultClock>();
        let (arqos_o, arqos_i) = signal::<U<4>, DefaultClock>();
        let (arvalid_o, arvalid_i) = signal::<Bit, DefaultClock>();
        let (rready_o, rready_i) = signal::<Bit, DefaultClock>();
        // What the controller drives and the unit reads.
        let (awready_o, awready_i) = signal::<Bit, DefaultClock>();
        let (wready_o, wready_i) = signal::<Bit, DefaultClock>();
        let (bid_o, bid_i) = signal::<U<5>, DefaultClock>();
        let (bresp_o, bresp_i) = signal::<U<2>, DefaultClock>();
        let (bvalid_o, bvalid_i) = signal::<Bit, DefaultClock>();
        let (arready_o, arready_i) = signal::<Bit, DefaultClock>();
        let (rid_o, rid_i) = signal::<U<5>, DefaultClock>();
        let (rdata_o, rdata_i) = signal::<U<32>, DefaultClock>();
        let (rresp_o, rresp_i) = signal::<U<2>, DefaultClock>();
        let (rlast_o, rlast_i) = signal::<Bit, DefaultClock>();
        let (rvalid_o, rvalid_i) = signal::<Bit, DefaultClock>();
        // The reset, read by both pins parts.
        let rst_wide = rst.clone();
        // The wide read port's pins, each way (issue 1696).
        let (xarid_o, xarid_i) = signal::<U<5>, DefaultClock>();
        let (xaraddr_o, xaraddr_i) = signal::<U<32>, DefaultClock>();
        let (xarlen_o, xarlen_i) = signal::<U<8>, DefaultClock>();
        let (xarvalid_o, xarvalid_i) = signal::<Bit, DefaultClock>();
        let (xrready_o, xrready_i) = signal::<Bit, DefaultClock>();
        let (xarready_o, xarready_i) = signal::<Bit, DefaultClock>();
        let (xrid_o, xrid_i) = signal::<U<5>, DefaultClock>();
        let (xrdata_o, xrdata_i) = signal::<U<256, 2>, DefaultClock>();
        let (xrresp_o, xrresp_i) = signal::<U<2>, DefaultClock>();
        let (xrlast_o, xrlast_i) = signal::<Bit, DefaultClock>();
        let (xrvalid_o, xrvalid_i) = signal::<Bit, DefaultClock>();
        // The controller first: in simulation it is the model, whose
        // pins the unit reads in the same step.
        join2(
            join2(
                self.ctl.run(
                    (
                        sys_clk, sys_rst, awid_i, awaddr_i, awlen_i, awsize_i,
                        awburst_i, awlock_i, awcache_i, awprot_i, awqos_i,
                        awvalid_i, wdata_i, wstrb_i, wlast_i, wvalid_i,
                        bready_i, arid_i, araddr_i, arlen_i, arsize_i,
                        arburst_i, arlock_i, arcache_i, arprot_i, arqos_i,
                        arvalid_i, rready_i, xarid_i, xaraddr_i, xarlen_i,
                        xarvalid_i, xrready_i,
                    ),
                    (
                        awready_o, wready_o, bid_o, bresp_o, bvalid_o,
                        arready_o, rid_o, rdata_o, rresp_o, rlast_o, rvalid_o,
                        xarready_o, xrid_o, xrdata_o, xrresp_o, xrlast_o,
                        xrvalid_o, calib, ui_clk, ui_rst, ck_p, ck_n,
                        mem_rst_n, cke, cs_n, ras_n, cas_n, we_n, row, bank,
                        dm, odt, dq, dqs, dqs_n,
                    ),
                ),
                self.pins.run(
                    AxiPerPinsIn {
                        rst,
                        pins: AxiPerDriven {
                            awready: awready_i,
                            wready: wready_i,
                            bid: bid_i,
                            bresp: bresp_i,
                            bvalid: bvalid_i,
                            arready: arready_i,
                            rid: rid_i,
                            rdata: rdata_i,
                            rresp: rresp_i,
                            rlast: rlast_i,
                            rvalid: rvalid_i,
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
                        awsize: awsize_o,
                        awburst: awburst_o,
                        awlock: awlock_o,
                        awcache: awcache_o,
                        awprot: awprot_o,
                        awqos: awqos_o,
                        awvalid: awvalid_o,
                        wdata: wdata_o,
                        wstrb: wstrb_o,
                        wlast: wlast_o,
                        wvalid: wvalid_o,
                        bready: bready_o,
                        arid: arid_o,
                        araddr: araddr_o,
                        arlen: arlen_o,
                        arsize: arsize_o,
                        arburst: arburst_o,
                        arlock: arlock_o,
                        arcache: arcache_o,
                        arprot: arprot_o,
                        arqos: arqos_o,
                        arvalid: arvalid_o,
                        rready: rready_o,
                    },
                ),
            ),
            self.wide.run(
                (
                    rst_wide, xar, xarready_i, xrid_i, xrdata_i, xrresp_i,
                    xrlast_i, xrvalid_i,
                ),
                (xr, xarid_o, xaraddr_o, xarlen_o, xarvalid_o, xrready_o),
            ),
        )
        .await;
    }
}
// end{per}

/// The peripheral on a link, driven from client code, and its netlist.
#[cfg(test)]
mod tests {
    use super::sim::WideRead;
    use super::{Ddr3Per, Ddr3PerBus, Ddr3PerPins, MODEL_WARMUP};
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;
    use txhdl::comp::{chan, join2, pad, signal, DefaultClock, Running, Unit};
    use txhdl::types::{Bit, U};
    use txhdl_parts::bus::axi::{axi, Ar, AxiHost, Link, Rd, Resp, Wr, R};

    /// Words written across the memory and read back, the first ones
    /// issued while the controller is still calibrating, and a burst
    /// that streams.
    #[test]
    fn words_go_in_and_come_back() {
        let Link {
            host,
            host_in,
            host_out,
            per_in,
            per_out,
            ..
        } = axi::<32, 32, 4, 5, 32>();
        let (aw, ar, w, _, _) = per_in;
        let (_, _, b, r) = per_out;
        let (_sys_clk_o, sys_clk) = signal::<Bit, DefaultClock>();
        let (_sys_rst_o, sys_rst) = signal::<Bit, DefaultClock>();
        let (calib_o, calib) = signal::<Bit, DefaultClock>();
        let bits = || signal::<Bit, DefaultClock>().0;
        let mut h = AxiHost::<32, 32, 4, 5, 32>::default();
        let mut mem = Ddr3Per::default();
        let seen = Rc::new(RefCell::new(Vec::new()));
        let out = seen.clone();
        let client = async move {
            let words =
                [(0x4000_0000u32, 1u32), (0x4000_0104, 2), (0x7fff_fffc, 3)];
            for (at, v) in words {
                let a = host.write(Wr::at(at), &[U::from(v)]).await;
                assert_eq!(a.done().await.resp, Resp::Okay);
            }
            for (at, _) in words {
                let r = host.read(Rd::at(at, 1)).await.done().await;
                out.borrow_mut().push(r.data[0].raw() as u32);
            }
            let burst: Vec<U<32>> = (0..16u32).map(U::from).collect();
            let a = host.write(Wr::at(0x4000_1000u32), &burst).await;
            assert_eq!(a.done().await.resp, Resp::Okay);
            let r = host.read(Rd::at(0x4000_1000u32, 16)).await.done().await;
            assert_eq!(r.data, burst, "the burst read back");
        };
        let mut sim = Running::new(join2(
            h.run(host_in, host_out),
            join2(
                mem.run(
                    Ddr3PerBus {
                        rst: signal::<Bit, DefaultClock>().1,
                        aw,
                        ar,
                        w,
                        b,
                        r,
                        xar: chan::<Ar<32, 5>, DefaultClock>().1,
                        xr: chan::<R<256, 5, 2>, DefaultClock>().0,
                    },
                    Ddr3PerPins {
                        sys_clk,
                        sys_rst,
                        calib: calib_o,
                        ui_clk: bits(),
                        ui_rst: bits(),
                        ck_p: bits(),
                        ck_n: bits(),
                        mem_rst_n: bits(),
                        cke: bits(),
                        cs_n: bits(),
                        ras_n: bits(),
                        cas_n: bits(),
                        we_n: bits(),
                        row: signal::<U<15>, DefaultClock>().0,
                        bank: signal::<U<3>, DefaultClock>().0,
                        dm: signal::<U<4>, DefaultClock>().0,
                        odt: bits(),
                        dq: pad::<U<32>, DefaultClock>(),
                        dqs: pad::<U<4>, DefaultClock>(),
                        dqs_n: pad::<U<4>, DefaultClock>(),
                    },
                ),
                client,
            ),
        ));
        let mut calibrated_at = None;
        for c in 0..2000u32 {
            sim.cycle();
            if calibrated_at.is_none() && calib.get().to_bool() {
                calibrated_at = Some(c);
            }
        }
        assert!(
            calibrated_at.unwrap_or(0) >= MODEL_WARMUP - 1,
            "calibrated at {calibrated_at:?}"
        );
        assert_eq!(*seen.borrow(), vec![1, 2, 3], "the words read back");
    }

    /// Words written through the 32-bit port and read back through the
    /// wide one, which shares the model's memory: a burst of two beats
    /// from an address inside the first, each beat eight words with the
    /// lowest address in bits 31:0, one beat a step (issue 1696).
    #[test]
    fn a_wide_read_streams_what_was_written() {
        let Link {
            host,
            host_in,
            host_out,
            per_in,
            per_out,
            ..
        } = axi::<32, 32, 4, 5, 32>();
        let (aw, ar, w, _, _) = per_in;
        let (_, _, b, r) = per_out;
        let bits = || signal::<Bit, DefaultClock>().0;
        let mut h = AxiHost::<32, 32, 4, 5, 32>::default();
        let mut mem = Ddr3Per::default();
        let mut wide = WideRead::<5>::on(mem.ctl.memory());
        let (wide_ar, wide_ar_rx) = chan::<Ar<32, 5>, DefaultClock>();
        let (wide_r_tx, wide_r) = chan::<R<256, 5, 2>, DefaultClock>();
        let now = Rc::new(Cell::new(0u32));
        let got = Rc::new(RefCell::new(Vec::new()));
        let (at, into) = (now.clone(), got.clone());
        let words: Vec<U<32>> =
            (0..16u32).map(|k| U::from(0x1000_0000 + k)).collect();
        let burst = words.clone();
        let client = async move {
            let a = host.write(Wr::at(0x4000_1000u32), &burst).await;
            assert_eq!(a.done().await.resp, Resp::Okay);
            wide_ar
                .put(|| Ar {
                    id: U::new(3),
                    addr: U::new(0x4000_1004),
                    len: U::new(1),
                    ..Default::default()
                })
                .await;
        };
        let sink = async move {
            loop {
                let beat = wide_r.wait().await;
                into.borrow_mut().push((at.get(), beat));
            }
        };
        let mut sim = Running::new(join2(
            h.run(host_in, host_out),
            join2(
                mem.run(
                    Ddr3PerBus {
                        rst: signal::<Bit, DefaultClock>().1,
                        aw,
                        ar,
                        w,
                        b,
                        r,
                        xar: chan::<Ar<32, 5>, DefaultClock>().1,
                        xr: chan::<R<256, 5, 2>, DefaultClock>().0,
                    },
                    Ddr3PerPins {
                        sys_clk: signal::<Bit, DefaultClock>().1,
                        sys_rst: signal::<Bit, DefaultClock>().1,
                        calib: bits(),
                        ui_clk: bits(),
                        ui_rst: bits(),
                        ck_p: bits(),
                        ck_n: bits(),
                        mem_rst_n: bits(),
                        cke: bits(),
                        cs_n: bits(),
                        ras_n: bits(),
                        cas_n: bits(),
                        we_n: bits(),
                        row: signal::<U<15>, DefaultClock>().0,
                        bank: signal::<U<3>, DefaultClock>().0,
                        dm: signal::<U<4>, DefaultClock>().0,
                        odt: bits(),
                        dq: pad::<U<32>, DefaultClock>(),
                        dqs: pad::<U<4>, DefaultClock>(),
                        dqs_n: pad::<U<4>, DefaultClock>(),
                    },
                ),
                join2(client, join2(wide.run(wide_ar_rx, wide_r_tx), sink)),
            ),
        ));
        for c in 0..2000u32 {
            now.set(c);
            sim.cycle();
        }
        let got = got.borrow();
        assert_eq!(got.len(), 2, "two beats");
        assert_eq!(got[1].0, got[0].0 + 1, "one beat a step");
        for (k, (_, beat)) in got.iter().enumerate() {
            let mut limbs = [0u128; 2];
            for j in 0..8 {
                limbs[j / 4] |= words[8 * k + j].raw() << (32 * (j % 4));
            }
            assert_eq!(beat.data.limbs(), limbs, "beat {k}");
            assert_eq!(beat.id.raw(), 3);
            assert_eq!(beat.resp, Resp::Okay);
            assert_eq!(beat.last.to_bool(), k == 1);
        }
    }

    /// The netlist holds the controller as an instance of the wrapper,
    /// its clock pin on the design's clock, the clock it makes and the
    /// board's clock among the peripheral's ports beside the pads, and
    /// no module of its own; the pins part is written, as a lowered
    /// child is.
    #[test]
    fn the_controller_is_instantiated_and_not_written() {
        let v = Ddr3Per::verilog("ddr3_per");
        assert!(v.contains("ddr3_axi32 "), "{v}");
        assert!(v.contains(".i_ui_clk(clk)"), "{v}");
        assert!(v.contains(".i_sys_clk(sys_clk)"), "{v}");
        assert!(v.contains(".o_ui_clk(ui_clk)"), "{v}");
        assert!(v.contains("inout [31:0] dq"), "{v}");
        assert!(v.contains(".io_ddr3_dq(dq)"), "{v}");
        assert!(v.contains("module ddr3_per_pins("), "{v}");
        assert!(!v.contains("module ddr3_axi32"), "{v}");
        let h = Ddr3Per::vhdl("ddr3_per");
        assert!(h.contains("component ddr3_axi32"), "{h}");
        assert!(
            h.contains("dq : inout std_logic_vector(31 downto 0)"),
            "{h}"
        );
        assert!(!h.contains("entity ddr3_axi32"), "{h}");
    }
}
